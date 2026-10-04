use crate::{
    CoreError,
    backups::keys::{KeyRef, WrappedDek},
    dataplane::{
        cloud_provider::Provider,
        credential::{CloudCredential, CloudCredentialId},
        value_objects::DataPlaneId,
    },
    organisation::OrganisationId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedSecret {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub wrapped_dek: WrappedDek,
    pub key: KeyRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedCredential {
    pub id: CloudCredentialId,
    pub organisation_id: OrganisationId,
    pub provider: Provider,
    pub sealed: SealedSecret,
}

pub trait SealedSecretRepository: Send + Sync {
    fn insert(
        &self,
        credential: &SealedCredential,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn get(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<Option<SealedCredential>, CoreError>> + Send;

    fn delete(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait CloudCredentialRepository: Send + Sync {
    fn insert(
        &self,
        credential: &CloudCredential,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn get(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<Option<CloudCredential>, CoreError>> + Send;

    fn list_for_organisation(
        &self,
        organisation_id: &OrganisationId,
    ) -> impl Future<Output = Result<Vec<CloudCredential>, CoreError>> + Send;

    fn is_in_use(
        &self,
        id: &CloudCredentialId,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}

pub trait DataPlaneFailures: Send + Sync {
    fn mark_failed(
        &self,
        id: &DataPlaneId,
        reason: &str,
    ) -> impl Future<Output = Result<bool, CoreError>> + Send;
}
