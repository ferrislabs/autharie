use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};

use super::{
    commands::{SmtpInput, SmtpOutcome},
    entities::{SmtpState, SmtpView, StoredSmtp},
    value_objects::SmtpRealm,
};
use crate::{CoreError, deployments::DeploymentId, organisation::OrganisationId};

pub trait SmtpPolicy: Send + Sync {
    fn can_view_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn can_manage_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait SmtpRepository: Send + Sync {
    fn next_revision(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;

    fn revision(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;

    fn get(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> impl Future<Output = Result<Option<StoredSmtp>, CoreError>> + Send;

    fn upsert(&self, stored: &StoredSmtp) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn delete(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn set_state(
        &self,
        deployment_id: DeploymentId,
        realm: &SmtpRealm,
        state: &SmtpState,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait SmtpService: Send + Sync {
    fn get_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> impl Future<Output = Result<Option<SmtpView>, CoreError>> + Send;

    fn save_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: SmtpInput,
    ) -> impl Future<Output = Result<SmtpView, CoreError>> + Send;

    fn remove_smtp(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        realm: SmtpRealm,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn branding_realms(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> impl Future<Output = Result<Vec<String>, CoreError>> + Send;
}

pub trait SmtpOutcomeService: Send + Sync {
    fn report_smtp_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: SmtpOutcome,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
