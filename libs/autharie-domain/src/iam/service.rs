use autharie_auth::Identity;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    IAM_ACTION_TYPE, IAM_REQUESTED_AUDIT, IamError, MAX_OUTSTANDING_REQUESTS, REQUEST_LIFETIME,
    entities::{IamFetch, IamInput, IamOutcome, IamRealm, IamRequest, IamState},
    operation::{IamAccess, IamOperation},
    payload::IamActionPayload,
    ports::{Clock, IamOutcomeService, IamPolicy, IamRepository, IamService, SystemClock},
};
use crate::{
    CoreError,
    action::{
        ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    agents::registry::SealingKey,
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        ports::AuditRepository, service::audit_actor,
    },
    dataplane::{herald_identity::speaking_for, ports::DataPlaneRepository},
    deployments::{Deployment, DeploymentId, DeploymentKind, ports::DeploymentRepository},
    generate_uuid_v7,
    organisation::OrganisationId,
    user::ports::UserRepository,
    webhooks::ports::KnownRealms,
};

const AUDIT_TARGET: &str = "deployment_iam";

pub struct IamServiceImpl<D, I, R, A, AU, U, P, C = SystemClock>
where
    A: ActionRepository,
{
    deployments: D,
    iam: I,
    realms: R,
    actions: ActionServiceImpl<A>,
    audit: AU,
    users: U,
    policy: P,
    clock: C,
}

impl<D, I, R, A, AU, U, P> IamServiceImpl<D, I, R, A, AU, U, P>
where
    A: ActionRepository,
{
    pub fn new(
        deployments: D,
        iam: I,
        realms: R,
        actions: A,
        audit: AU,
        users: U,
        policy: P,
    ) -> Self {
        Self {
            deployments,
            iam,
            realms,
            actions: ActionServiceImpl::new(actions),
            audit,
            users,
            policy,
            clock: SystemClock,
        }
    }
}

impl<D, I, R, A, AU, U, P, C> IamServiceImpl<D, I, R, A, AU, U, P, C>
where
    A: ActionRepository,
{
    pub fn with_clock<K>(self, clock: K) -> IamServiceImpl<D, I, R, A, AU, U, P, K> {
        IamServiceImpl {
            deployments: self.deployments,
            iam: self.iam,
            realms: self.realms,
            actions: self.actions,
            audit: self.audit,
            users: self.users,
            policy: self.policy,
            clock,
        }
    }
}

impl<D, I, R, A, AU, U, P, C> IamServiceImpl<D, I, R, A, AU, U, P, C>
where
    D: DeploymentRepository,
    I: IamRepository,
    R: KnownRealms,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: IamPolicy,
    C: Clock,
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

    async fn ferriskey_deployment(
        &self,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Deployment, CoreError> {
        let deployment = self.deployment(organisation_id, deployment_id).await?;

        if deployment.kind != DeploymentKind::Ferriskey {
            return Err(IamError::UnsupportedKind.into());
        }

        Ok(deployment)
    }

    async fn check_realm(
        &self,
        deployment_id: DeploymentId,
        realm: &IamRealm,
    ) -> Result<(), CoreError> {
        let known = self.realms.known_realms(deployment_id).await?;

        if known.iter().any(|known| known == realm.as_str()) {
            return Ok(());
        }

        Err(IamError::invalid(format!(
            "the realm '{}' does not exist on this deployment",
            realm.as_str()
        ))
        .into())
    }

    async fn record(
        &self,
        deployment: &Deployment,
        request: &IamRequest,
        operation: &IamOperation,
        sealing_key: &SealingKey,
    ) -> Result<(), CoreError> {
        let data = serde_json::to_value(IamActionPayload::new(
            deployment.id.0,
            &deployment.namespace,
            request.id,
            request.realm.as_str(),
            operation,
            sealing_key.payload(),
            request.expires_at,
        ))
        .map_err(|error| CoreError::InternalError(error.to_string()))?;

        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(IAM_ACTION_TYPE.to_string()),
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

    async fn write_audit(
        &self,
        identity: &Identity,
        organisation_id: OrganisationId,
        request: &IamRequest,
    ) -> Result<(), CoreError> {
        self.audit
            .append(AuditEntry::record(
                AuditEntryId(generate_uuid_v7()),
                organisation_id,
                audit_actor(identity, &self.users).await?,
                AuditAction(IAM_REQUESTED_AUDIT.to_string()),
                AuditTarget {
                    kind: AuditTargetKind::Custom(AUDIT_TARGET.to_string()),
                    id: request.deployment_id.0,
                },
                Some(AuditChange::new(
                    Value::Null,
                    json!({
                        "request_id": request.id,
                        "realm": request.realm.as_str(),
                        "operation": request.operation.as_str(),
                    }),
                )?),
                request.created_at,
            ))
            .await
    }
}

impl<D, I, R, A, AU, U, P, C> IamService for IamServiceImpl<D, I, R, A, AU, U, P, C>
where
    D: DeploymentRepository,
    I: IamRepository,
    R: KnownRealms,
    A: ActionRepository,
    AU: AuditRepository,
    U: UserRepository,
    P: IamPolicy,
    C: Clock,
{
    async fn ask_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: IamInput,
    ) -> Result<Uuid, CoreError> {
        self.policy
            .can_access_iam(identity.clone(), organisation_id, input.operation.access())
            .await?;
        let deployment = self
            .ferriskey_deployment(organisation_id, deployment_id)
            .await?;
        self.check_realm(deployment_id, &input.realm).await?;

        let now = self.clock.now();
        let request = IamRequest {
            id: generate_uuid_v7(),
            deployment_id,
            realm: input.realm,
            operation: input.operation.kind(),
            actor: identity.id().to_string(),
            state: IamState::Pending,
            created_at: now,
            expires_at: now + REQUEST_LIFETIME,
        };

        if !self.iam.open(&request, MAX_OUTSTANDING_REQUESTS).await? {
            return Err(IamError::TooManyRequests {
                max: MAX_OUTSTANDING_REQUESTS,
            }
            .into());
        }

        self.record(&deployment, &request, &input.operation, &input.sealing_key)
            .await?;
        self.write_audit(&identity, organisation_id, &request)
            .await?;

        Ok(request.id)
    }

    async fn poll_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> Result<IamFetch, CoreError> {
        self.policy
            .can_access_iam(identity.clone(), organisation_id, IamAccess::Read)
            .await?;
        self.deployment(organisation_id, deployment_id).await?;

        let now = self.clock.now();
        let request = self
            .iam
            .get(deployment_id, request_id, now)
            .await?
            .filter(|request| request.actor == identity.id())
            .ok_or(IamError::NotFound { request_id })?;

        match request.state {
            IamState::Pending => Ok(IamFetch::Pending),
            IamState::Failed { message } => Err(IamError::Failed { message }.into()),
            IamState::Delivered => Err(IamError::Gone { request_id }.into()),
            IamState::Done => Ok(self
                .iam
                .take_envelope(deployment_id, request_id, now)
                .await?
                .map(IamFetch::Ready)
                .ok_or(IamError::Gone { request_id })?),
        }
    }
}

pub struct IamOutcomeServiceImpl<D, P, I, C = SystemClock> {
    deployments: D,
    dataplanes: P,
    iam: I,
    clock: C,
}

impl<D, P, I> IamOutcomeServiceImpl<D, P, I> {
    pub fn new(deployments: D, dataplanes: P, iam: I) -> Self {
        Self {
            deployments,
            dataplanes,
            iam,
            clock: SystemClock,
        }
    }
}

impl<D, P, I, C> IamOutcomeServiceImpl<D, P, I, C> {
    pub fn with_clock<K>(self, clock: K) -> IamOutcomeServiceImpl<D, P, I, K> {
        IamOutcomeServiceImpl {
            deployments: self.deployments,
            dataplanes: self.dataplanes,
            iam: self.iam,
            clock,
        }
    }
}

impl<D, P, I, C> IamOutcomeService for IamOutcomeServiceImpl<D, P, I, C>
where
    D: DeploymentRepository,
    P: DataPlaneRepository,
    I: IamRepository,
    C: Clock,
{
    async fn report_iam_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: IamOutcome,
    ) -> Result<bool, CoreError> {
        let speaking = speaking_for(&self.dataplanes, &identity).await?;

        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        speaking.is(deployment.dataplane_id)?;

        self.iam
            .record_outcome(
                deployment_id,
                outcome.request_id,
                &outcome.verdict,
                self.clock.now(),
            )
            .await
    }
}
