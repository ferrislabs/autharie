use std::future::Future;

use crate::{
    CoreError,
    dataplane::{
        credential::{CredentialError, SecretString},
        value_objects::Region,
    },
    deployments::DeploymentId,
};

use super::{Cell, CellId, RealmError, RealmName};

#[cfg_attr(test, mockall::automock)]
pub trait CellRepository: Send + Sync {
    fn place(
        &self,
        region: &Region,
    ) -> impl Future<Output = Result<Option<Cell>, CoreError>> + Send;

    fn release(&self, cell: CellId) -> impl Future<Output = Result<(), CoreError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait RealmAdmin: Send + Sync {
    fn create_realm(
        &self,
        cell: CellId,
        name: &RealmName,
        key: DeploymentId,
    ) -> impl Future<Output = Result<(), RealmError>> + Send;

    fn delete_realm(
        &self,
        cell: CellId,
        name: &RealmName,
    ) -> impl Future<Output = Result<(), RealmError>> + Send;
}

#[cfg_attr(test, mockall::automock)]
pub trait CellCredentialStore: Send + Sync {
    fn put(
        &self,
        cell: CellId,
        secret: SecretString,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send;

    fn get_for_admin(
        &self,
        cell: CellId,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    fn delete(&self, cell: CellId) -> impl Future<Output = Result<(), CredentialError>> + Send;
}
