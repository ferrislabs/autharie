use std::future::Future;

use autharie_crds::v1alpha::identity_instance::{IamRequestStatus, IdentityInstance};

use super::result::IamResult;
use super::{IamError, IamOperation};
use crate::domain::OperatorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IamOutcome {
    Settled,
    Retry,
}

pub trait IdentityInstanceIam: Send + Sync {
    fn reconcile(&self, instance: &IdentityInstance) -> impl Future<Output = IamOutcome> + Send;
}

pub trait FerriskeyIamPort: Send + Sync {
    fn execute(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        operation: &IamOperation,
    ) -> impl Future<Output = Result<IamResult, IamError>> + Send;
}

pub trait IamResultsPort: Send + Sync {
    fn put(
        &self,
        instance: &IdentityInstance,
        request_id: &str,
        envelope: &str,
    ) -> impl Future<Output = Result<(), IamError>> + Send;

    fn forget(
        &self,
        instance: &IdentityInstance,
        request_ids: &[&str],
    ) -> impl Future<Output = Result<(), IamError>> + Send;
}

pub trait IamStatusRepository: Send + Sync {
    fn recorded(
        &self,
        instance: &IdentityInstance,
    ) -> impl Future<Output = Result<Vec<IamRequestStatus>, OperatorError>> + Send;

    fn patch_requests(
        &self,
        instance: &IdentityInstance,
        requests: &[IamRequestStatus],
    ) -> impl Future<Output = Result<(), OperatorError>> + Send;
}
