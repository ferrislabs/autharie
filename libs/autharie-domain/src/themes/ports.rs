use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use super::{
    commands::{NewTheme, ThemeChange, ThemeOutcome},
    entities::{Theme, ThemeListing, ThemeState, ThemeSync},
    value_objects::{ThemeId, ThemeRealm},
};
use crate::{CoreError, deployments::DeploymentId, organisation::OrganisationId};

pub trait ThemePolicy: Send + Sync {
    fn can_view_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn can_manage_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait ThemeRepository: Send + Sync {
    fn lock_realm(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn list(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> impl Future<Output = Result<Vec<Theme>, CoreError>> + Send;

    fn get(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> impl Future<Output = Result<Option<Theme>, CoreError>> + Send;

    fn active(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> impl Future<Output = Result<Option<Theme>, CoreError>> + Send;

    fn insert(&self, theme: &Theme) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn update(&self, theme: &Theme) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn delete(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn activate(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn deactivate(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn next_revision(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;

    fn sync(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
    ) -> impl Future<Output = Result<Option<ThemeSync>, CoreError>> + Send;

    fn set_state(
        &self,
        deployment_id: DeploymentId,
        realm: &ThemeRealm,
        state: &ThemeState,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait ThemeService: Send + Sync {
    fn list_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> impl Future<Output = Result<ThemeListing, CoreError>> + Send;

    fn create_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        theme: NewTheme,
    ) -> impl Future<Output = Result<Theme, CoreError>> + Send;

    fn update_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
        change: ThemeChange,
    ) -> impl Future<Output = Result<Theme, CoreError>> + Send;

    fn delete_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn activate_theme(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
        id: ThemeId,
    ) -> impl Future<Output = Result<Theme, CoreError>> + Send;

    fn deactivate_themes(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: ThemeRealm,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait ThemeOutcomeService: Send + Sync {
    fn report_theme_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: ThemeOutcome,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
