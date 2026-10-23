use autharie_auth::Identity;
use chrono::Utc;

use super::{
    SMTP_ACTION_TYPE, SmtpError,
    commands::{SmtpInput, SmtpOutcome},
    entities::{SmtpConfig, SmtpState, SmtpView, StoredSmtp},
    payload::{SmtpActionPayload, SmtpActionSettings},
    ports::{SmtpOutcomeService, SmtpPolicy, SmtpRepository, SmtpService},
    value_objects::SmtpRealm,
};
use crate::{
    CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    dataplane::{herald_identity::speaking_for, ports::DataPlaneRepository},
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    organisation::OrganisationId,
    webhooks::ports::{KnownRealms, SecretVault},
};

fn password_context(deployment_id: DeploymentId, realm: &SmtpRealm) -> String {
    format!("smtp-password:{}:{}", deployment_id.0, realm.as_str())
}

pub struct SmtpServiceImpl<D, S, R, A, V, P>
where
    A: ActionRepository,
{
    deployments: D,
    smtp: S,
    realms: R,
    actions: ActionServiceImpl<A>,
    vault: V,
    policy: P,
}

impl<D, S, R, A, V, P> SmtpServiceImpl<D, S, R, A, V, P>
where
    A: ActionRepository,
{
    pub fn new(deployments: D, smtp: S, realms: R, actions: A, vault: V, policy: P) -> Self {
        Self {
            deployments,
            smtp,
            realms,
            actions: ActionServiceImpl::new(actions),
            vault,
            policy,
        }
    }
}

impl<D, S, R, A, V, P> SmtpServiceImpl<D, S, R, A, V, P>
where
    D: DeploymentRepository,
    S: SmtpRepository,
    R: KnownRealms,
    A: ActionRepository,
    V: SecretVault,
    P: SmtpPolicy,
{
    async fn deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        self.deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })
    }

    async fn managed_deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(SmtpError::UnsupportedKind.into());
        }

        Ok(deployment)
    }

    async fn check_realm(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<(), CoreError> {
        let known = self.realms.known_realms(deployment_id).await?;

        if known.iter().any(|known| known == realm.as_str()) {
            return Ok(());
        }

        Err(SmtpError::invalid(format!(
            "the realm '{}' does not exist on this deployment",
            realm.as_str()
        ))
        .into())
    }

    async fn record(
        &self,
        deployment: &Deployment,
        revision: u64,
        realm: &SmtpRealm,
        smtp: Option<SmtpActionSettings>,
    ) -> Result<(), CoreError> {
        let data = serde_json::to_value(SmtpActionPayload {
            deployment_id: deployment.id.0,
            namespace: deployment.namespace.clone(),
            revision,
            realm: realm.as_str().to_string(),
            smtp,
        })
        .map_err(|error| CoreError::InternalError(error.to_string()))?;

        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(SMTP_ACTION_TYPE.to_string()),
                ActionTarget {
                    kind: TargetKind::Deployment,
                    id: deployment.id.0,
                },
                ActionPayload { data },
                ActionVersion(1),
                ActionSource::System,
            ))
            .await?;

        Ok(())
    }
}

impl<D, S, R, A, V, P> SmtpService for SmtpServiceImpl<D, S, R, A, V, P>
where
    D: DeploymentRepository,
    S: SmtpRepository,
    R: KnownRealms,
    A: ActionRepository,
    V: SecretVault,
    P: SmtpPolicy,
{
    async fn get_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> Result<Option<SmtpView>, CoreError> {
        self.policy.can_view_smtp(identity, organisation_id).await?;
        self.deployment(organisation_id, deployment_id).await?;

        Ok(self
            .smtp
            .get(deployment_id, &realm)
            .await?
            .map(|stored| SmtpView::from(stored.config)))
    }

    async fn save_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: SmtpInput,
    ) -> Result<SmtpView, CoreError> {
        self.policy
            .can_manage_smtp(identity, organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;
        self.check_realm(deployment_id, &input.realm).await?;

        let sealed_password = match (
            &input.password,
            self.smtp.get(deployment_id, &input.realm).await?,
        ) {
            (Some(password), _) => {
                self.vault
                    .seal(
                        &password_context(deployment_id, &input.realm),
                        password.secret(),
                    )
                    .await?
            }
            (None, Some(stored)) => stored.sealed_password,
            (None, None) => return Err(SmtpError::PasswordRequired.into()),
        };

        let revision = self.smtp.next_revision(deployment_id, &input.realm).await?;

        let stored = StoredSmtp {
            config: SmtpConfig {
                deployment_id,
                realm: input.realm,
                settings: input.settings,
                state: SmtpState::Pending,
                updated_at: Utc::now(),
            },
            sealed_password,
        };
        self.smtp.upsert(&stored).await?;

        self.record(
            &deployment,
            revision,
            &stored.config.realm,
            Some(SmtpActionSettings::of(
                &stored.config.settings,
                input.password.as_ref().map(|password| password.expose()),
            )),
        )
        .await?;

        Ok(SmtpView::from(stored.config))
    }

    async fn remove_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> Result<(), CoreError> {
        self.policy
            .can_manage_smtp(identity, organisation_id)
            .await?;
        let deployment = self
            .managed_deployment(organisation_id, deployment_id)
            .await?;

        if !self.smtp.delete(deployment_id, &realm).await? {
            return Err(SmtpError::NotFound {
                realm: realm.as_str().to_string(),
            }
            .into());
        }

        let revision = self.smtp.next_revision(deployment_id, &realm).await?;

        self.record(&deployment, revision, &realm, None).await
    }

    async fn branding_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<String>, CoreError> {
        self.policy.can_view_smtp(identity, organisation_id).await?;
        self.deployment(organisation_id, deployment_id).await?;

        self.realms.known_realms(deployment_id).await
    }
}

pub struct SmtpOutcomeServiceImpl<D, P, S> {
    deployments: D,
    dataplanes: P,
    smtp: S,
}

impl<D, P, S> SmtpOutcomeServiceImpl<D, P, S> {
    pub fn new(deployments: D, dataplanes: P, smtp: S) -> Self {
        Self {
            deployments,
            dataplanes,
            smtp,
        }
    }
}

impl<D, P, S> SmtpOutcomeService for SmtpOutcomeServiceImpl<D, P, S>
where
    D: DeploymentRepository,
    P: DataPlaneRepository,
    S: SmtpRepository,
{
    async fn report_smtp_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: SmtpOutcome,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&self.dataplanes, &identity).await?;

        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;

        if outcome.revision < self.smtp.revision(deployment_id, &outcome.realm).await? {
            return Ok(false);
        }

        let Some(stored) = self.smtp.get(deployment_id, &outcome.realm).await? else {
            return Ok(false);
        };
        if stored.config.state == outcome.state {
            return Ok(false);
        }

        self.smtp
            .set_state(deployment_id, &outcome.realm, &outcome.state, Utc::now())
            .await
    }
}
