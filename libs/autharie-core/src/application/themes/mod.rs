use autharie_auth::Identity;
use autharie_domain::themes::{
    NewTheme, ThemeChange, ThemeId, ThemeListing, ThemeOutcome, ThemeRealm,
    ports::{ThemeOutcomeService, ThemePolicy, ThemeService},
    service::{ThemeOutcomeServiceImpl, ThemeServiceImpl},
};
use autharie_domain::{
    CoreError, deployments::DeploymentId, organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_macros::transactional;
use autharie_permission::Permissions;

pub use autharie_domain::themes::{
    HexColour, MAX_THEMES_PER_REALM, PORTAL_THEME_ACTION_TYPE, PortalThemeActionPayload, Theme,
    ThemeError, ThemeName, ThemeState, ThemeSync, ThemeTokens, ThemeView, commands, entities,
    parse_realm, payload, ports, service, tokens, value_objects,
};

use crate::{AutharieService, infrastructure::role::permissions_in, policy::PolicyContext};

struct InstanceRights<R> {
    permissions: R,
}

impl<R> ThemePolicy for InstanceRights<R>
where
    R: PermissionProvider,
{
    async fn can_view_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::VIEW_INSTANCES)
    }

    async fn can_manage_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<(), CoreError> {
        let permissions = self
            .permissions
            .permissions_for_organisation(identity, organisation_id)
            .await?;

        PolicyContext::new(permissions).require_permission(Permissions::MANAGE_INSTANCES)
    }
}

impl ThemeService for AutharieService {
    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn list_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> Result<ThemeListing, CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .list_themes(identity, organisation_id, deployment_id, realm)
        .await
    }

    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn create_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        theme: NewTheme,
    ) -> Result<Theme, CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .create_theme(identity, organisation_id, deployment_id, realm, theme)
        .await
    }

    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn update_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
        change: ThemeChange,
    ) -> Result<Theme, CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .update_theme(identity, organisation_id, deployment_id, realm, id, change)
        .await
    }

    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn delete_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> Result<(), CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .delete_theme(identity, organisation_id, deployment_id, realm, id)
        .await
    }

    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn activate_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> Result<Theme, CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .activate_theme(identity, organisation_id, deployment_id, realm, id)
        .await
    }

    #[transactional(deployment, theme, webhook, subscription, action)]
    async fn deactivate_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> Result<(), CoreError> {
        ThemeServiceImpl::new(
            deployment_repository,
            theme_repository,
            webhook_repository,
            action_repository,
            subscription_repository,
            InstanceRights {
                permissions: permissions_in(&tx),
            },
        )
        .deactivate_themes(identity, organisation_id, deployment_id, realm)
        .await
    }
}

impl ThemeOutcomeService for AutharieService {
    #[transactional(data_plane, deployment, theme)]
    async fn report_theme_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: ThemeOutcome,
    ) -> Result<bool, CoreError> {
        ThemeOutcomeServiceImpl::new(
            deployment_repository,
            data_plane_repository,
            theme_repository,
        )
        .report_theme_outcome(identity, deployment_id, outcome)
        .await
    }
}
