use autharie_auth::Identity;
use autharie_macros::transactional;

use crate::{
    CoreError,
    application::AutharieService,
    infrastructure::role::permissions_in,
    organisation::service::OrganisationServiceImpl,
    organisation::{
        Organisation, OrganisationId,
        commands::{CreateOrganisationCommand, UpdateOrganisationCommand},
        ports::OrganisationRepository,
        ports::OrganisationService,
        ports::{OrganisationMcpSwitch, OrganisationSettings},
    },
    policy::{AuthariePolicy, PolicyContext},
};
use autharie_domain::agents::service::AgentPublisher;
use autharie_domain::role::ports::PermissionProvider;
use autharie_permission::Permissions;

impl OrganisationService for AutharieService {
    #[transactional(organisation, user)]
    async fn create_organisation(
        &self,
        command: CreateOrganisationCommand,
    ) -> Result<Organisation, CoreError> {
        OrganisationServiceImpl::new(organisation_repository, user_repository)
            .create_organisation(command)
            .await
    }

    #[transactional(organisation, user)]
    async fn delete_organisation(&self, id: OrganisationId) -> Result<(), CoreError> {
        OrganisationServiceImpl::new(organisation_repository, user_repository)
            .delete_organisation(id)
            .await
    }

    #[transactional(organisation, user)]
    async fn update_organisation(
        &self,
        id: OrganisationId,
        command: UpdateOrganisationCommand,
    ) -> Result<Organisation, CoreError> {
        OrganisationServiceImpl::new(organisation_repository, user_repository)
            .update_organisation(id, command)
            .await
    }

    #[transactional(organisation, user)]
    async fn get_organisations_by_member(
        &self,
        identity: Identity,
    ) -> Result<Vec<Organisation>, CoreError> {
        OrganisationServiceImpl::new(organisation_repository, user_repository)
            .get_organisations_by_member(identity)
            .await
    }
}

impl OrganisationSettings for AutharieService {
    #[transactional(organisation)]
    async fn get_organisation(
        &self,
        identity: Identity,
        id: OrganisationId,
    ) -> Result<Organisation, CoreError> {
        let granted = permissions_in(&tx)
            .permissions_for_organisation(identity, id)
            .await?;
        PolicyContext::new(granted).require_any(&[
            Permissions::VIEW_ORGANISATION,
            Permissions::MANAGE_ORGANISATION,
        ])?;

        organisation_repository
            .find_by_id(&id)
            .await?
            .ok_or(CoreError::OrganisationNotFound { id: id.0 })
    }

    #[transactional(organisation, user, deployment, agent, action)]
    async fn change_organisation(
        &self,
        identity: Identity,
        id: OrganisationId,
        command: UpdateOrganisationCommand,
    ) -> Result<Organisation, CoreError> {
        let granted = permissions_in(&tx)
            .permissions_for_organisation(identity, id)
            .await?;
        PolicyContext::new(granted).require_permission(Permissions::MANAGE_ORGANISATION)?;

        let before = organisation_repository
            .find_by_id(&id)
            .await?
            .map(|organisation| organisation.mcp_enabled);

        let updated = OrganisationServiceImpl::new(organisation_repository, user_repository)
            .update_organisation(id, command)
            .await?;

        if before.is_some_and(|before| before != updated.mcp_enabled) {
            AgentPublisher::new(deployment_repository, agent_repository, action_repository)
                .republish_organisation(id, updated.mcp_enabled)
                .await?;
        }

        Ok(updated)
    }
}

impl OrganisationMcpSwitch for AutharieService {
    #[transactional(organisation)]
    async fn mcp_enabled(&self, id: OrganisationId) -> Result<bool, CoreError> {
        Ok(organisation_repository
            .find_by_id(&id)
            .await?
            .is_some_and(|organisation| organisation.is_active() && organisation.mcp_enabled))
    }
}

impl autharie_domain::offers::ports::OfferService for AutharieService {
    // Only the organisation: what a tier opens is read from its plan, and
    // asking for a repository this never touches is a repository the next
    // reader has to work out is unused.
    #[transactional(organisation)]
    async fn list_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<autharie_domain::offers::OfferAvailability>, CoreError> {
        // Gated before the organisation is read: the answer discloses which
        // tier it is on, which is not a stranger's business.
        autharie_domain::deployments::ports::DeploymentPolicy::can_view_deployments(
            &AuthariePolicy::new(permissions_in(&tx)),
            identity,
            organisation_id,
        )
        .await?;

        let organisation = organisation_repository
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })?;

        Ok(autharie_domain::offers::offers_for(organisation.plan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::organisation::value_objects::{OrganisationName, Plan};
    use autharie_auth::Identity;
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use uuid::Uuid;

    fn service() -> AutharieService {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://user:pass@127.0.0.1:1/db")
            .expect("valid database url");
        AutharieService::new(pool)
    }

    #[tokio::test]
    async fn create_organisation_maps_pool_error() {
        let command = CreateOrganisationCommand::new(
            OrganisationName::new("Acme Corp").unwrap(),
            "user-sub-1".to_string(),
            Plan::Free,
        );

        let result = service().create_organisation(command).await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn get_organisations_by_member_rejects_invalid_identity() {
        let identity = Identity::User(autharie_auth::User {
            scopes: Vec::new(),
            act: None,
            id: "not-a-uuid".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        });

        let result = service().get_organisations_by_member(identity).await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn get_organisation_maps_pool_error() {
        let identity = Identity::User(autharie_auth::User {
            scopes: Vec::new(),
            act: None,
            id: "user-1".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        });

        let result = service()
            .get_organisation(identity, OrganisationId(Uuid::new_v4()))
            .await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn change_organisation_maps_pool_error() {
        let identity = Identity::User(autharie_auth::User {
            scopes: Vec::new(),
            act: None,
            id: "user-1".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        });

        let result = service()
            .change_organisation(
                identity,
                OrganisationId(Uuid::new_v4()),
                UpdateOrganisationCommand::new().with_mcp_enabled(true),
            )
            .await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn mcp_enabled_maps_pool_error() {
        let result = service().mcp_enabled(OrganisationId(Uuid::new_v4())).await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn delete_organisation_maps_pool_error() {
        let result = service()
            .delete_organisation(OrganisationId(Uuid::new_v4()))
            .await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }

    #[tokio::test]
    async fn update_organisation_maps_pool_error() {
        let command = UpdateOrganisationCommand::new();
        let result = service()
            .update_organisation(OrganisationId(Uuid::new_v4()), command)
            .await;
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
    }
}
