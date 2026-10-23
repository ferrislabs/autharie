use autharie_auth::Identity;
use chrono::Utc;
use uuid::Uuid;

use super::{
    MAX_THEMES_PER_REALM, PORTAL_THEME_ACTION_TYPE, ThemeError,
    commands::{NewTheme, ThemeChange, ThemeOutcome},
    entities::{Theme, ThemeListing},
    payload::PortalThemeActionPayload,
    ports::{ThemeOutcomeService, ThemePolicy, ThemeRepository, ThemeService},
    tokens::ThemeTokens,
    value_objects::{ThemeId, ThemeRealm},
};
use crate::{
    CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    billing::ports::SubscriptionRepository,
    dataplane::{herald_identity::speaking_for, ports::DataPlaneRepository},
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    organisation::{OrganisationId, features::IamFeature},
    webhooks::ports::KnownRealms,
};

pub struct ThemeServiceImpl<D, T, R, A, S, P>
where
    A: ActionRepository,
{
    deployments: D,
    themes: T,
    realms: R,
    actions: ActionServiceImpl<A>,
    subscriptions: S,
    policy: P,
}

impl<D, T, R, A, S, P> ThemeServiceImpl<D, T, R, A, S, P>
where
    A: ActionRepository,
{
    pub fn new(
        deployments: D,
        themes: T,
        realms: R,
        actions: A,
        subscriptions: S,
        policy: P,
    ) -> Self {
        Self {
            deployments,
            themes,
            realms,
            actions: ActionServiceImpl::new(actions),
            subscriptions,
            policy,
        }
    }
}

impl<D, T, R, A, S, P> ThemeServiceImpl<D, T, R, A, S, P>
where
    D: DeploymentRepository,
    T: ThemeRepository,
    R: KnownRealms,
    A: ActionRepository,
    S: SubscriptionRepository,
    P: ThemePolicy,
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

    async fn require_branding(&self, deployment_id: DeploymentId) -> Result<(), CoreError> {
        let subscription = self
            .subscriptions
            .latest_for(&deployment_id)
            .await?
            .ok_or_else(|| CoreError::PermissionDenied {
                reason: "this deployment has no subscription".into(),
            })?;
        let terms = subscription.terms();

        if !terms.opens(IamFeature::Branding) {
            return Err(CoreError::PermissionDenied {
                reason: format!(
                    "the {} tier does not open {}",
                    terms.tier,
                    IamFeature::Branding
                ),
            });
        }

        Ok(())
    }

    async fn managed(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Deployment, CoreError> {
        self.policy
            .can_manage_themes(identity, organisation_id)
            .await?;
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(ThemeError::UnsupportedKind.into());
        }

        self.require_branding(deployment_id).await?;
        self.themes.lock_realm(deployment_id, realm).await?;

        Ok(deployment)
    }

    async fn found(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> Result<Theme, CoreError> {
        self.themes
            .get(deployment_id, realm, id)
            .await?
            .ok_or_else(|| {
                ThemeError::NotFound {
                    id: id.to_string(),
                    realm: realm.as_str().to_string(),
                }
                .into()
            })
    }

    async fn send(
        &self,
        deployment: &Deployment,
        realm: &ThemeRealm,
        config: ThemeTokens,
    ) -> Result<(), CoreError> {
        let revision = self.themes.next_revision(deployment.id, realm).await?;

        let data = serde_json::to_value(PortalThemeActionPayload {
            deployment_id: deployment.id.0,
            namespace: deployment.namespace.clone(),
            revision,
            realm: realm.as_str().to_string(),
            config,
        })
        .map_err(|error| CoreError::InternalError(error.to_string()))?;

        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(PORTAL_THEME_ACTION_TYPE.to_string()),
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

impl<D, T, R, A, S, P> ThemeService for ThemeServiceImpl<D, T, R, A, S, P>
where
    D: DeploymentRepository,
    T: ThemeRepository,
    R: KnownRealms,
    A: ActionRepository,
    S: SubscriptionRepository,
    P: ThemePolicy,
{
    async fn list_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> Result<ThemeListing, CoreError> {
        self.policy
            .can_view_themes(identity, organisation_id)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        Ok(ThemeListing {
            themes: self.themes.list(deployment_id, &realm).await?,
            state: self.themes.sync(deployment_id, &realm).await?,
        })
    }

    async fn create_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        theme: NewTheme,
    ) -> Result<Theme, CoreError> {
        self.managed(identity, organisation_id, deployment_id, &realm)
            .await?;

        let known = self.realms.known_realms(deployment_id).await?;
        if !known.iter().any(|known| known == realm.as_str()) {
            return Err(ThemeError::invalid(format!(
                "the realm '{}' does not exist on this deployment",
                realm.as_str()
            ))
            .into());
        }

        if self.themes.list(deployment_id, &realm).await?.len() >= MAX_THEMES_PER_REALM {
            return Err(ThemeError::ThemeLimit.into());
        }

        let at = Utc::now();
        let created = Theme {
            id: ThemeId(Uuid::new_v4()),
            deployment_id,
            realm,
            name: theme.name,
            tokens: theme.tokens,
            active: false,
            created_at: at,
            updated_at: at,
        };
        self.themes.insert(&created).await?;

        Ok(created)
    }

    async fn update_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
        change: ThemeChange,
    ) -> Result<Theme, CoreError> {
        let deployment = self
            .managed(identity, organisation_id, deployment_id, &realm)
            .await?;
        let mut theme = self.found(deployment_id, &realm, id).await?;

        let tokens_changed = change
            .tokens
            .as_ref()
            .is_some_and(|tokens| *tokens != theme.tokens);

        if let Some(name) = change.name {
            theme.name = name;
        }
        if let Some(tokens) = change.tokens {
            theme.tokens = tokens;
        }
        theme.updated_at = Utc::now();
        self.themes.update(&theme).await?;

        if theme.active && tokens_changed {
            self.send(&deployment, &realm, theme.tokens.clone()).await?;
        }

        Ok(theme)
    }

    async fn delete_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> Result<(), CoreError> {
        self.managed(identity, organisation_id, deployment_id, &realm)
            .await?;
        let theme = self.found(deployment_id, &realm, id).await?;

        if theme.active {
            return Err(ThemeError::ActiveTheme.into());
        }

        self.themes.delete(deployment_id, &realm, id).await?;

        Ok(())
    }

    async fn activate_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> Result<Theme, CoreError> {
        let deployment = self
            .managed(identity, organisation_id, deployment_id, &realm)
            .await?;
        let mut theme = self.found(deployment_id, &realm, id).await?;

        let at = Utc::now();
        if !self.themes.activate(deployment_id, &realm, id, at).await? {
            return Err(ThemeError::NotFound {
                id: id.to_string(),
                realm: realm.as_str().to_string(),
            }
            .into());
        }
        theme.active = true;
        theme.updated_at = at;

        self.send(&deployment, &realm, theme.tokens.clone()).await?;

        Ok(theme)
    }

    async fn deactivate_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> Result<(), CoreError> {
        let deployment = self
            .managed(identity, organisation_id, deployment_id, &realm)
            .await?;

        self.themes
            .deactivate(deployment_id, &realm, Utc::now())
            .await?;

        self.send(&deployment, &realm, ThemeTokens::default()).await
    }
}

pub struct ThemeOutcomeServiceImpl<D, P, T> {
    deployments: D,
    dataplanes: P,
    themes: T,
}

impl<D, P, T> ThemeOutcomeServiceImpl<D, P, T> {
    pub fn new(deployments: D, dataplanes: P, themes: T) -> Self {
        Self {
            deployments,
            dataplanes,
            themes,
        }
    }
}

impl<D, P, T> ThemeOutcomeService for ThemeOutcomeServiceImpl<D, P, T>
where
    D: DeploymentRepository,
    P: DataPlaneRepository,
    T: ThemeRepository,
{
    async fn report_theme_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: ThemeOutcome,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&self.dataplanes, &identity).await?;

        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;

        let Some(sync) = self.themes.sync(deployment_id, &outcome.realm).await? else {
            return Ok(false);
        };
        if outcome.revision < sync.revision || sync.state == outcome.state {
            return Ok(false);
        }

        self.themes
            .set_state(deployment_id, &outcome.realm, &outcome.state)
            .await
    }
}
