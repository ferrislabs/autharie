use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    dataplane::{
        cloud_provider::{
            CatalogError, ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId,
            CredentialVerifier, Money, NodeType, Provider, ProviderCatalog, ProviderOffers,
        },
        cluster_profile::{
            ClusterMode, ClusterProfile, CostEstimate, ProfileError, Replication,
            estimate_cluster_cost,
        },
        credential::{
            CloudCredential, CloudCredentialId, CloudCredentialStore, CredentialError, SecretString,
        },
        credential_repository::CloudCredentialRepository,
        provisioner::ProvisionError,
        value_objects::Region,
    },
    deployments::distribution::Distribution,
    organisation::OrganisationId,
    role::ports::PermissionProvider,
};
use autharie_permission::Permissions;
use chrono::Utc;
use tracing::warn;

use crate::policy::PolicyContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSpec {
    pub mode: ClusterMode,
    pub control_plane_id: ControlPlaneOfferId,
    pub node_type: NodeType,
    pub min_nodes: u8,
    pub max_nodes: u8,
    pub replication: Replication,
}

impl ProfileSpec {
    pub fn into_profile(self, offers: &ProviderOffers) -> Result<ClusterProfile, ProfileError> {
        let control_plane = offers
            .control_plane(&self.control_plane_id)
            .cloned()
            .unwrap_or(ControlPlaneOffer {
                id: self.control_plane_id,
                kind: ControlPlaneKind::Mutualized,
                monthly_price: Money::ZERO,
            });

        ClusterProfile::new(
            self.mode,
            control_plane,
            self.node_type,
            self.min_nodes,
            self.max_nodes,
            self.replication,
            offers,
        )
    }
}

async fn require<P: PermissionProvider>(
    permissions: &P,
    identity: Identity,
    organisation_id: OrganisationId,
    needed: Permissions,
) -> Result<(), CoreError> {
    let granted = permissions
        .permissions_for_organisation(identity, organisation_id)
        .await?;

    PolicyContext::new(granted).require_permission(needed)
}

async fn owned_credential<R: CloudCredentialRepository>(
    repository: &R,
    id: &CloudCredentialId,
    organisation_id: OrganisationId,
) -> Result<CloudCredential, CoreError> {
    repository
        .get(id)
        .await?
        .filter(|credential| credential.organisation_id == organisation_id)
        .ok_or_else(|| CredentialError::NotFound { id: *id }.into())
}

fn catalog_failure(error: CatalogError) -> CoreError {
    match error {
        CatalogError::CredentialRejected => ProvisionError::CredentialRejected.into(),
        CatalogError::RegionUnavailable { .. } => ProvisionError::RegionUnavailable.into(),
        CatalogError::Unavailable(reason) => CoreError::ProvisioningUnavailable {
            reason: format!("The provider catalog could not be read: {reason}"),
        },
    }
}

async fn offers_for<C: ProviderCatalog>(
    catalog: &C,
    credential: &CloudCredential,
    region: &Region,
) -> Result<ProviderOffers, CoreError> {
    catalog
        .offers(credential.provider, &credential.id, region)
        .await
        .map_err(catalog_failure)
}

pub struct RegisterCloudCredential<'a, S, V, R, P> {
    store: S,
    verifier: &'a V,
    repository: R,
    permissions: P,
}

impl<'a, S, V, R, P> RegisterCloudCredential<'a, S, V, R, P>
where
    S: CloudCredentialStore,
    V: CredentialVerifier,
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(store: S, verifier: &'a V, repository: R, permissions: P) -> Self {
        Self {
            store,
            verifier,
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        provider: Provider,
        label: String,
        secret: SecretString,
    ) -> Result<CloudCredential, CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::MANAGE_ORGANISATION,
        )
        .await?;

        let scope_check = self.verifier.verify(provider, &secret).await?;
        let id = self.store.put(organisation_id, provider, secret).await?;

        let credential = CloudCredential {
            id,
            organisation_id,
            provider,
            label,
            scope_check,
            created_at: Utc::now(),
        };

        if let Err(error) = self.repository.insert(&credential).await {
            if let Err(cleanup) = self.store.delete(&id).await {
                warn!(credential = %id, %cleanup, "a stored secret could not be removed after its record failed");
            }
            return Err(error);
        }

        Ok(credential)
    }
}

pub struct ListCloudCredentials<R, P> {
    repository: R,
    permissions: P,
}

impl<R, P> ListCloudCredentials<R, P>
where
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(repository: R, permissions: P) -> Self {
        Self {
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<CloudCredential>, CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::VIEW_ORGANISATION,
        )
        .await?;

        self.repository
            .list_for_organisation(&organisation_id)
            .await
    }
}

pub struct DeleteCloudCredential<S, R, P> {
    store: S,
    repository: R,
    permissions: P,
}

impl<S, R, P> DeleteCloudCredential<S, R, P>
where
    S: CloudCredentialStore,
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(store: S, repository: R, permissions: P) -> Self {
        Self {
            store,
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
    ) -> Result<(), CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::MANAGE_ORGANISATION,
        )
        .await?;

        owned_credential(&self.repository, &id, organisation_id).await?;

        if self.repository.is_in_use(&id).await? {
            return Err(CredentialError::InUse.into());
        }

        self.store.delete(&id).await?;
        Ok(())
    }
}

pub struct ListProviderOffers<'a, C, R, P> {
    catalog: &'a C,
    repository: R,
    permissions: P,
}

impl<'a, C, R, P> ListProviderOffers<'a, C, R, P>
where
    C: ProviderCatalog,
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(catalog: &'a C, repository: R, permissions: P) -> Self {
        Self {
            catalog,
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: &Region,
    ) -> Result<ProviderOffers, CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::CREATE_INSTANCES,
        )
        .await?;

        let credential = owned_credential(&self.repository, &id, organisation_id).await?;
        offers_for(self.catalog, &credential, region).await
    }
}

pub struct EstimateClusterCost<'a, C, R, P> {
    catalog: &'a C,
    repository: R,
    permissions: P,
}

impl<'a, C, R, P> EstimateClusterCost<'a, C, R, P>
where
    C: ProviderCatalog,
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(catalog: &'a C, repository: R, permissions: P) -> Self {
        Self {
            catalog,
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: &Region,
        spec: ProfileSpec,
    ) -> Result<CostEstimate, CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::CREATE_INSTANCES,
        )
        .await?;

        let credential = owned_credential(&self.repository, &id, organisation_id).await?;
        let offers = offers_for(self.catalog, &credential, region).await?;
        let profile = spec.into_profile(&offers)?;

        Ok(estimate_cluster_cost(&profile, &offers)?)
    }
}

pub struct ResolveCustomerCloud<'a, C, R, P> {
    catalog: &'a C,
    repository: R,
    permissions: P,
}

impl<'a, C, R, P> ResolveCustomerCloud<'a, C, R, P>
where
    C: ProviderCatalog,
    R: CloudCredentialRepository,
    P: PermissionProvider,
{
    pub fn new(catalog: &'a C, repository: R, permissions: P) -> Self {
        Self {
            catalog,
            repository,
            permissions,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        id: CloudCredentialId,
        region: &Region,
        spec: ProfileSpec,
    ) -> Result<Distribution, CoreError> {
        require(
            &self.permissions,
            identity,
            organisation_id,
            Permissions::CREATE_INSTANCES,
        )
        .await?;

        let credential = owned_credential(&self.repository, &id, organisation_id).await?;
        let offers = offers_for(self.catalog, &credential, region).await?;

        Ok(Distribution::CustomerCloud {
            credential_id: credential.id,
            profile: spec.into_profile(&offers)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use autharie_domain::dataplane::{cloud_provider::NodeOffer, credential::ScopeCheck};
    use uuid::Uuid;

    use super::*;
    use crate::infrastructure::credentials::{CloudProviders, FixedCloudProvider, FixedVerdict};

    struct Grants(Permissions);

    impl PermissionProvider for Grants {
        async fn permissions_for_organisation(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<Permissions, CoreError> {
            Ok(self.0)
        }
    }

    #[derive(Default)]
    struct Store {
        put: Mutex<Vec<(OrganisationId, Provider, String)>>,
        deleted: Mutex<Vec<CloudCredentialId>>,
        last: Mutex<Option<CloudCredentialId>>,
    }

    impl CloudCredentialStore for &Store {
        async fn put(
            &self,
            organisation_id: OrganisationId,
            provider: Provider,
            secret: SecretString,
        ) -> Result<CloudCredentialId, CredentialError> {
            let id = CloudCredentialId(Uuid::new_v4());
            self.put.lock().expect("lock").push((
                organisation_id,
                provider,
                secret.expose().to_string(),
            ));
            *self.last.lock().expect("lock") = Some(id);
            Ok(id)
        }

        async fn get_for_provisioning(
            &self,
            id: &CloudCredentialId,
        ) -> Result<SecretString, CredentialError> {
            Err(CredentialError::NotFound { id: *id })
        }

        async fn delete(&self, id: &CloudCredentialId) -> Result<(), CredentialError> {
            self.deleted.lock().expect("lock").push(*id);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Repository {
        rows: Mutex<Vec<CloudCredential>>,
        in_use: bool,
        fail_insert: bool,
    }

    impl CloudCredentialRepository for &Repository {
        async fn insert(&self, credential: &CloudCredential) -> Result<(), CoreError> {
            if self.fail_insert {
                return Err(CoreError::DatabaseError {
                    message: "down".to_string(),
                });
            }
            self.rows.lock().expect("lock").push(credential.clone());
            Ok(())
        }

        async fn get(&self, id: &CloudCredentialId) -> Result<Option<CloudCredential>, CoreError> {
            Ok(self
                .rows
                .lock()
                .expect("lock")
                .iter()
                .find(|row| &row.id == id)
                .cloned())
        }

        async fn list_for_organisation(
            &self,
            organisation_id: &OrganisationId,
        ) -> Result<Vec<CloudCredential>, CoreError> {
            Ok(self
                .rows
                .lock()
                .expect("lock")
                .iter()
                .filter(|row| &row.organisation_id == organisation_id)
                .cloned()
                .collect())
        }

        async fn is_in_use(&self, _id: &CloudCredentialId) -> Result<bool, CoreError> {
            Ok(self.in_use)
        }
    }

    fn identity() -> Identity {
        Identity::User(autharie_auth::User {
            id: "user".to_string(),
            username: "user".to_string(),
            email: None,
            name: None,
            roles: vec![],
        })
    }

    fn organisation() -> OrganisationId {
        OrganisationId(Uuid::new_v4())
    }

    fn offers() -> ProviderOffers {
        ProviderOffers {
            control_planes: vec![
                ControlPlaneOffer {
                    id: ControlPlaneOfferId::new("mutualized"),
                    kind: ControlPlaneKind::Mutualized,
                    monthly_price: Money::ZERO,
                },
                ControlPlaneOffer {
                    id: ControlPlaneOfferId::new("dedicated-4"),
                    kind: ControlPlaneKind::Dedicated,
                    monthly_price: Money::new(7_000),
                },
            ],
            node_types: vec![NodeOffer {
                node_type: NodeType::new("small"),
                monthly_price: Money::new(1_000),
            }],
        }
    }

    fn provider(verdict: FixedVerdict) -> CloudProviders {
        CloudProviders::Fixed(FixedCloudProvider {
            offers: offers(),
            verdict,
        })
    }

    fn spec(
        mode: ClusterMode,
        control_plane: &str,
        node: &str,
        min: u8,
        max: u8,
        replicas: u8,
    ) -> ProfileSpec {
        ProfileSpec {
            mode,
            control_plane_id: ControlPlaneOfferId::new(control_plane),
            node_type: NodeType::new(node),
            min_nodes: min,
            max_nodes: max,
            replication: Replication::new(replicas).expect("replicas"),
        }
    }

    fn all() -> Grants {
        Grants(Permissions::ADMINISTRATOR)
    }

    fn credential(organisation_id: OrganisationId) -> CloudCredential {
        CloudCredential {
            id: CloudCredentialId(Uuid::new_v4()),
            organisation_id,
            provider: Provider::Scaleway,
            label: "production".to_string(),
            scope_check: ScopeCheck {
                checked_at: Utc::now(),
            },
            created_at: Utc::now(),
        }
    }

    fn repository_holding(credential: &CloudCredential, in_use: bool) -> Repository {
        Repository {
            rows: Mutex::new(vec![credential.clone()]),
            in_use,
            fail_insert: false,
        }
    }

    #[tokio::test]
    async fn a_credential_with_the_required_permissions_is_registered_spec_ccp_1() {
        let (store, repository) = (Store::default(), Repository::default());
        let verifier = provider(FixedVerdict::Accept);
        let organisation_id = organisation();

        let registered = RegisterCloudCredential::new(&store, &verifier, &repository, all())
            .execute(
                identity(),
                organisation_id,
                Provider::Scaleway,
                "production".to_string(),
                SecretString::new("key"),
            )
            .await
            .expect("registered");

        assert_eq!(registered.organisation_id, organisation_id);
        assert_eq!(registered.label, "production");
        assert_eq!(repository.rows.lock().expect("lock").len(), 1);
        assert_eq!(store.put.lock().expect("lock")[0].2, "key");
    }

    #[tokio::test]
    async fn a_credential_missing_permissions_is_refused_with_the_list_and_never_stored_spec_ccp_2()
    {
        let (store, repository) = (Store::default(), Repository::default());
        let verifier = provider(FixedVerdict::Missing(vec![
            "KubernetesFullAccess".to_string(),
        ]));

        let error = RegisterCloudCredential::new(&store, &verifier, &repository, all())
            .execute(
                identity(),
                organisation(),
                Provider::Scaleway,
                "production".to_string(),
                SecretString::new("key"),
            )
            .await
            .expect_err("refused");

        assert!(matches!(
            &error,
            CoreError::Credential(CredentialError::MissingPermissions { missing })
                if missing == &["KubernetesFullAccess"]
        ));
        assert!(store.put.lock().expect("lock").is_empty());
        assert!(repository.rows.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn a_credential_with_excess_permissions_is_refused_with_the_list_spec_ccp_2() {
        let (store, repository) = (Store::default(), Repository::default());
        let verifier = provider(FixedVerdict::Excess(vec!["IAMManager".to_string()]));

        let error = RegisterCloudCredential::new(&store, &verifier, &repository, all())
            .execute(
                identity(),
                organisation(),
                Provider::Scaleway,
                "production".to_string(),
                SecretString::new("key"),
            )
            .await
            .expect_err("refused");

        assert!(matches!(
            &error,
            CoreError::Credential(CredentialError::ExcessPermissions { extra })
                if extra == &["IAMManager"]
        ));
        assert!(store.put.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn the_secret_is_not_in_the_refusal_spec_ccp_3() {
        let (store, repository) = (Store::default(), Repository::default());
        let verifier = provider(FixedVerdict::Invalid);

        let error = RegisterCloudCredential::new(&store, &verifier, &repository, all())
            .execute(
                identity(),
                organisation(),
                Provider::Scaleway,
                "production".to_string(),
                SecretString::new("hunter2-secret"),
            )
            .await
            .expect_err("refused");

        assert!(!format!("{error} {error:?}").contains("hunter2-secret"));
    }

    #[tokio::test]
    async fn registering_needs_the_right_to_manage_the_organisation() {
        let (store, repository) = (Store::default(), Repository::default());
        let verifier = provider(FixedVerdict::Accept);

        let error = RegisterCloudCredential::new(
            &store,
            &verifier,
            &repository,
            Grants(Permissions::VIEW_ORGANISATION | Permissions::CREATE_INSTANCES),
        )
        .execute(
            identity(),
            organisation(),
            Provider::Scaleway,
            "production".to_string(),
            SecretString::new("key"),
        )
        .await
        .expect_err("refused");

        assert!(matches!(error, CoreError::PermissionDenied { .. }));
        assert!(store.put.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn a_secret_whose_record_failed_is_removed_again() {
        let store = Store::default();
        let repository = Repository {
            fail_insert: true,
            ..Repository::default()
        };
        let verifier = provider(FixedVerdict::Accept);

        let result = RegisterCloudCredential::new(&store, &verifier, &repository, all())
            .execute(
                identity(),
                organisation(),
                Provider::Scaleway,
                "production".to_string(),
                SecretString::new("key"),
            )
            .await;

        let stored = store
            .last
            .lock()
            .expect("lock")
            .expect("a secret was stored");
        assert!(matches!(result, Err(CoreError::DatabaseError { .. })));
        assert_eq!(*store.deleted.lock().expect("lock"), vec![stored]);
    }

    #[tokio::test]
    async fn credentials_are_listed_for_their_organisation_only() {
        let mine = organisation();
        let repository = repository_holding(&credential(mine), false);
        repository
            .rows
            .lock()
            .expect("lock")
            .push(credential(organisation()));

        let listed = ListCloudCredentials::new(&repository, all())
            .execute(identity(), mine)
            .await
            .expect("listed");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].organisation_id, mine);
    }

    #[tokio::test]
    async fn a_credential_in_use_cannot_be_deleted_spec_ccp_4() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let (store, repository) = (Store::default(), repository_holding(&held, true));

        let error = DeleteCloudCredential::new(&store, &repository, all())
            .execute(identity(), organisation_id, held.id)
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            CoreError::Credential(CredentialError::InUse)
        ));
        assert!(store.deleted.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn an_unused_credential_is_deleted() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let (store, repository) = (Store::default(), repository_holding(&held, false));

        DeleteCloudCredential::new(&store, &repository, all())
            .execute(identity(), organisation_id, held.id)
            .await
            .expect("deleted");

        assert_eq!(*store.deleted.lock().expect("lock"), vec![held.id]);
    }

    #[tokio::test]
    async fn another_organisations_credential_is_not_found_rather_than_deleted() {
        let held = credential(organisation());
        let (store, repository) = (Store::default(), repository_holding(&held, false));

        let error = DeleteCloudCredential::new(&store, &repository, all())
            .execute(identity(), organisation(), held.id)
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            CoreError::Credential(CredentialError::NotFound { .. })
        ));
        assert!(store.deleted.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn offers_are_read_through_the_catalog_for_a_credential_of_the_organisation() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);

        let listed = ListProviderOffers::new(&catalog, &repository, all())
            .execute(identity(), organisation_id, held.id, &Region::new("fr-par"))
            .await
            .expect("offers");

        assert_eq!(listed, offers());
    }

    #[tokio::test]
    async fn offers_are_refused_for_a_credential_of_another_organisation() {
        let held = credential(organisation());
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);

        let error = ListProviderOffers::new(&catalog, &repository, all())
            .execute(identity(), organisation(), held.id, &Region::new("fr-par"))
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            CoreError::Credential(CredentialError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn an_unreadable_catalog_is_reported_not_swallowed() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = CloudProviders::Unconfigured;

        let error = ListProviderOffers::new(&catalog, &repository, all())
            .execute(identity(), organisation_id, held.id, &Region::new("fr-par"))
            .await
            .expect_err("refused");

        assert!(matches!(error, CoreError::ProvisioningUnavailable { .. }));
    }

    #[tokio::test]
    async fn the_estimated_cost_is_shown_for_a_valid_profile_spec_ccp_8() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);

        let estimate = EstimateClusterCost::new(&catalog, &repository, all())
            .execute(
                identity(),
                organisation_id,
                held.id,
                &Region::new("fr-par"),
                spec(ClusterMode::Ha, "dedicated-4", "small", 3, 10, 2),
            )
            .await
            .expect("estimated");

        assert_eq!(estimate.min, Money::new(3 * 1_000 + 7_000));
        assert_eq!(estimate.max, Money::new(10 * 1_000 + 7_000));
    }

    #[tokio::test]
    async fn a_refused_profile_is_not_estimated() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);
        let estimator = EstimateClusterCost::new(&catalog, &repository, all());
        let region = Region::new("fr-par");

        let dev_with_two = estimator
            .execute(
                identity(),
                organisation_id,
                held.id,
                &region,
                spec(ClusterMode::Dev, "mutualized", "small", 2, 2, 1),
            )
            .await
            .expect_err("refused");
        let unknown_node = estimator
            .execute(
                identity(),
                organisation_id,
                held.id,
                &region,
                spec(ClusterMode::Standard, "mutualized", "gigantic", 2, 4, 2),
            )
            .await
            .expect_err("refused");
        let unknown_control_plane = estimator
            .execute(
                identity(),
                organisation_id,
                held.id,
                &region,
                spec(ClusterMode::Standard, "dedicated-64", "small", 2, 4, 2),
            )
            .await
            .expect_err("refused");
        let dev_dedicated = estimator
            .execute(
                identity(),
                organisation_id,
                held.id,
                &region,
                spec(ClusterMode::Dev, "dedicated-4", "small", 1, 1, 1),
            )
            .await
            .expect_err("refused");

        assert!(matches!(
            dev_with_two,
            CoreError::Profile(ProfileError::AboveModeCeiling { .. })
        ));
        assert!(matches!(
            unknown_node,
            CoreError::Profile(ProfileError::NodeTypeUnavailable { .. })
        ));
        assert!(matches!(
            unknown_control_plane,
            CoreError::Profile(ProfileError::ControlPlaneUnavailable { .. })
        ));
        assert!(matches!(
            dev_dedicated,
            CoreError::Profile(ProfileError::ControlPlaneNotAllowedForMode { .. })
        ));
    }

    #[tokio::test]
    async fn a_customer_cloud_distribution_carries_the_credential_and_the_checked_profile() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);

        let distribution = ResolveCustomerCloud::new(&catalog, &repository, all())
            .execute(
                identity(),
                organisation_id,
                held.id,
                &Region::new("fr-par"),
                spec(ClusterMode::Standard, "dedicated-4", "small", 2, 5, 2),
            )
            .await
            .expect("resolved");

        let Distribution::CustomerCloud {
            credential_id,
            profile,
        } = distribution
        else {
            panic!("not a customer cloud distribution");
        };
        assert_eq!(credential_id, held.id);
        assert_eq!(profile.control_plane().monthly_price, Money::new(7_000));
        assert_eq!((profile.min_nodes(), profile.max_nodes()), (2, 5));
    }

    #[tokio::test]
    async fn estimating_and_resolving_need_the_right_to_create_instances() {
        let organisation_id = organisation();
        let held = credential(organisation_id);
        let repository = repository_holding(&held, false);
        let catalog = provider(FixedVerdict::Accept);
        let viewer = || Grants(Permissions::VIEW_ORGANISATION);

        let estimate = EstimateClusterCost::new(&catalog, &repository, viewer())
            .execute(
                identity(),
                organisation_id,
                held.id,
                &Region::new("fr-par"),
                spec(ClusterMode::Dev, "mutualized", "small", 1, 1, 1),
            )
            .await;
        let resolved = ResolveCustomerCloud::new(&catalog, &repository, viewer())
            .execute(
                identity(),
                organisation_id,
                held.id,
                &Region::new("fr-par"),
                spec(ClusterMode::Dev, "mutualized", "small", 1, 1, 1),
            )
            .await;
        let offered = ListProviderOffers::new(&catalog, &repository, viewer())
            .execute(identity(), organisation_id, held.id, &Region::new("fr-par"))
            .await;

        assert!(matches!(estimate, Err(CoreError::PermissionDenied { .. })));
        assert!(matches!(resolved, Err(CoreError::PermissionDenied { .. })));
        assert!(matches!(offered, Err(CoreError::PermissionDenied { .. })));
    }

    #[test]
    fn a_catalog_failure_keeps_its_reason() {
        assert!(matches!(
            catalog_failure(CatalogError::CredentialRejected),
            CoreError::Provision(ProvisionError::CredentialRejected)
        ));
        assert!(matches!(
            catalog_failure(CatalogError::RegionUnavailable {
                region: "nl-ams".to_string()
            }),
            CoreError::Provision(ProvisionError::RegionUnavailable)
        ));
    }
}
