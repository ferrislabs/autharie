use std::future::Future;

use chrono::{DateTime, Utc};

use crate::{CoreError, dataplane::value_objects::Region, deployments::DeploymentId};

use super::{Cell, CellId, CellStatus};

#[cfg_attr(test, mockall::automock)]
pub trait CellRepository: Send + Sync {
    fn create(&self, cell: &Cell) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn find(&self, id: CellId) -> impl Future<Output = Result<Option<Cell>, CoreError>> + Send;

    fn find_by_instance(
        &self,
        instance: DeploymentId,
    ) -> impl Future<Output = Result<Option<Cell>, CoreError>> + Send;

    fn list(&self) -> impl Future<Output = Result<Vec<Cell>, CoreError>> + Send;

    fn set_status(
        &self,
        id: CellId,
        status: CellStatus,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn place(
        &self,
        region: &Region,
    ) -> impl Future<Output = Result<Option<Cell>, CoreError>> + Send;

    fn release(
        &self,
        deployment: DeploymentId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn release_refused(
        &self,
        deployment: DeploymentId,
        reason: &str,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn is_instance_hostname(
        &self,
        hostname_slug: &str,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;

    fn realm_refusal(
        &self,
        deployment: DeploymentId,
    ) -> impl Future<Output = Result<Option<String>, CoreError>> + Send;

    fn stalled_realms(
        &self,
        quiet_since: DateTime<Utc>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<DeploymentId>, CoreError>> + Send;
}
