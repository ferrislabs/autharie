mod resize;
mod resizer;
mod service;

use autharie_auth::Identity;
use autharie_domain::audit::service::AuditServiceImpl;
use autharie_domain::{
    CoreError,
    dataplane::{
        cloud_provider::{Provider, ProviderOffers},
        cluster_profile::{ClusterProfile, CostEstimate},
        credential::{CloudCredential, CloudCredentialId, CredentialError, SecretString},
        value_objects::Region,
    },
    deployments::{DeploymentId, distribution::Distribution},
    organisation::OrganisationId,
};
use autharie_macros::transactional;

pub use resize::ResizeCustomerCluster;
pub use resizer::{ClusterResizers, RecordingResizer, ResizeOnly};
pub use service::{
    DeleteCloudCredential, EstimateClusterCost, ListCloudCredentials, ListProviderOffers,
    ProfileSpec, RegisterCloudCredential, ResolveCustomerCloud,
};

use crate::{
    AutharieService,
    infrastructure::{credentials::EnvelopeCredentialStore, role::permissions_in},
    policy::AuthariePolicy,
};

pub trait CloudProviderService: Send + Sync {
    fn register_cloud_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        provider: Provider,
        label: String,
        secret: SecretString,
    ) -> impl Future<Output = Result<CloudCredential, CoreError>> + Send;

    fn list_cloud_credentials(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> impl Future<Output = Result<Vec<CloudCredential>, CoreError>> + Send;

    fn delete_cloud_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    fn list_provider_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
    ) -> impl Future<Output = Result<ProviderOffers, CoreError>> + Send;

    fn estimate_cluster_cost(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
        spec: ProfileSpec,
    ) -> impl Future<Output = Result<CostEstimate, CoreError>> + Send;

    fn resolve_customer_cloud(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
        spec: ProfileSpec,
    ) -> impl Future<Output = Result<Distribution, CoreError>> + Send;

    fn resize_customer_cluster(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        spec: ProfileSpec,
    ) -> impl Future<Output = Result<ClusterProfile, CoreError>> + Send;
}

impl CloudProviderService for AutharieService {
    #[transactional(cloud_credential, sealed_secret)]
    async fn register_cloud_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        provider: Provider,
        label: String,
        secret: SecretString,
    ) -> Result<CloudCredential, CoreError> {
        let keys = self.credential_keys().ok_or_else(|| {
            CredentialError::Store("no key manager is configured on this installation".to_string())
        })?;

        RegisterCloudCredential::new(
            EnvelopeCredentialStore::new(sealed_secret_repository, keys)?,
            self.cloud_providers(),
            cloud_credential_repository,
            permissions_in(&tx),
        )
        .execute(identity, organisation_id, provider, label, secret)
        .await
    }

    #[transactional(cloud_credential)]
    async fn list_cloud_credentials(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<CloudCredential>, CoreError> {
        ListCloudCredentials::new(cloud_credential_repository, permissions_in(&tx))
            .execute(identity, organisation_id)
            .await
    }

    #[transactional(cloud_credential, sealed_secret)]
    async fn delete_cloud_credential(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
    ) -> Result<(), CoreError> {
        let keys = self.credential_keys().ok_or_else(|| {
            CredentialError::Store("no key manager is configured on this installation".to_string())
        })?;

        DeleteCloudCredential::new(
            EnvelopeCredentialStore::new(sealed_secret_repository, keys)?,
            cloud_credential_repository,
            permissions_in(&tx),
        )
        .execute(identity, organisation_id, id)
        .await
    }

    #[transactional(cloud_credential)]
    async fn list_provider_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
    ) -> Result<ProviderOffers, CoreError> {
        ListProviderOffers::new(
            self.cloud_providers(),
            cloud_credential_repository,
            permissions_in(&tx),
        )
        .execute(identity, organisation_id, id, &region)
        .await
    }

    #[transactional(cloud_credential)]
    async fn estimate_cluster_cost(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
        spec: ProfileSpec,
    ) -> Result<CostEstimate, CoreError> {
        EstimateClusterCost::new(
            self.cloud_providers(),
            cloud_credential_repository,
            permissions_in(&tx),
        )
        .execute(identity, organisation_id, id, &region, spec)
        .await
    }

    #[transactional(cloud_credential)]
    async fn resolve_customer_cloud(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: Region,
        spec: ProfileSpec,
    ) -> Result<Distribution, CoreError> {
        ResolveCustomerCloud::new(
            self.cloud_providers(),
            cloud_credential_repository,
            permissions_in(&tx),
        )
        .execute(identity, organisation_id, id, &region, spec)
        .await
    }

    #[transactional(cloud_credential, deployment, data_plane, audit, user)]
    async fn resize_customer_cluster(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        spec: ProfileSpec,
    ) -> Result<ClusterProfile, CoreError> {
        ResizeCustomerCluster::new(
            self.cloud_providers(),
            self.cluster_resizers(),
            deployment_repository,
            data_plane_repository,
            cloud_credential_repository,
            AuthariePolicy::new(permissions_in(&tx)),
            AuditServiceImpl::new(audit_repository, permissions_in(&tx)),
            user_repository,
        )
        .execute(identity, organisation_id, deployment_id, spec)
        .await
    }
}
