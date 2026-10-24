use std::future::Future;

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    entities::{IamFetch, IamInput, IamOutcome, IamRequest, IamVerdict},
    envelope::Envelope,
    operation::IamAccess,
};
use crate::{CoreError, deployments::DeploymentId, organisation::OrganisationId};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub trait IamPolicy: Send + Sync {
    fn can_access_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        access: IamAccess,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

pub trait IamRepository: Send + Sync {
    fn open(
        &self,
        request: &IamRequest,
        max_outstanding: usize,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn get(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<IamRequest>, CoreError>> + Send;

    fn record_outcome(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        verdict: &IamVerdict,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn take_envelope(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<Envelope>, CoreError>> + Send;

    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<u64, CoreError>> + Send;
}

pub trait IamService: Send + Sync {
    fn ask_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        input: IamInput,
    ) -> impl Future<Output = Result<Uuid, CoreError>> + Send;

    fn poll_iam(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        request_id: Uuid,
    ) -> impl Future<Output = Result<IamFetch, CoreError>> + Send;
}

pub trait IamOutcomeService: Send + Sync {
    fn report_iam_outcome(
        &self,
        identity: Identity,
        deployment_id: DeploymentId,
        outcome: IamOutcome,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
