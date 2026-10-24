use autharie_auth::Identity;
use autharie_domain::iam::{
    IamAccess, IamFetch, IamInput, IamOutcome,
    ports::{IamOutcomeService, IamPolicy, IamRepository, IamService},
    service::{IamOutcomeServiceImpl, IamServiceImpl},
};
use autharie_domain::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;
use uuid::Uuid;

pub use autharie_domain::iam::{
    Envelope, IAM_ACTION_TYPE, IAM_REQUESTED_AUDIT, IamActionPayload, IamError, IamOperation,
    IamOperationKind, IamRealm, IamRequest, IamState, IamVerdict, MAX_ENVELOPE_BYTES,
    MAX_OUTSTANDING_REQUESTS, MAX_PAGE_SIZE, Page, PageSize, REQUEST_LIFETIME, SearchTerm,
    entities, envelope, operation, payload, ports, redacted, service,
};

use crate::{AutharieService, infrastructure::role::permissions_in, policy::PolicyContext};

struct InstanceRights<R> {
    permissions: R,
}

impl<R> IamPolicy for InstanceRights<R>
where
    R: PermissionProvider,
{
    async fn can_access_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        access: IamAccess,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        let required = match access {
            IamAccess::Read => Permissions::VIEW_INSTANCES,
            IamAccess::Write => Permissions::MANAGE_INSTANCES,
        };

        PolicyContext::new(permissions).require_permission(required)
    }
}

impl AutharieService {
    #[transactional(iam)]
    pub async fn purge_expired_iam_requests(&self) -> Result<u64, CoreError> {
        iam_repository.purge_expired(chrono::Utc::now()).await
    }
}

impl IamService for AutharieService {
    #[transactional(deployment, iam, webhook, action, audit, user)]
    async fn ask_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: IamInput,
    ) -> Result<Uuid, CoreError> {
        IamServiceImpl::new(
            deployment_repository,
            iam_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .ask_iam(identity, organisation_id, deployment_id, input)
        .await
    }

    #[transactional(deployment, iam, webhook, action, audit, user)]
    async fn poll_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> Result<IamFetch, CoreError> {
        IamServiceImpl::new(
            deployment_repository,
            iam_repository,
            webhook_repository,
            action_repository,
            audit_repository,
            user_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .poll_iam(identity, organisation_id, deployment_id, request_id)
        .await
    }
}

impl IamOutcomeService for AutharieService {
    #[transactional(data_plane, deployment, iam)]
    async fn report_iam_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: IamOutcome,
    ) -> Result<bool, CoreError> {
        IamOutcomeServiceImpl::new(deployment_repository, data_plane_repository, iam_repository)
            .report_iam_outcome(identity, deployment_id, outcome)
            .await
    }
}
