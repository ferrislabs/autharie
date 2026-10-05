use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    audit::{
        AuditAction, AuditActor, AuditChange, AuditTarget, AuditTargetKind,
        commands::RecordAuditEntryCommand, ports::AuditService, service::audit_actor,
    },
    dataplane::{
        cloud_provider::{ProviderCatalog, ProviderOffers},
        cluster_profile::{ClusterProfile, ResizeError},
        credential_repository::CloudCredentialRepository,
        entities::DataPlane,
        ports::DataPlaneRepository,
        provisioner::ClusterProvisioner,
        value_objects::DataPlaneStatus,
    },
    deployments::{
        Deployment, DeploymentId, DeploymentStatus,
        distribution::Distribution,
        ports::{DeploymentPolicy, DeploymentRepository},
    },
    organisation::OrganisationId,
    user::ports::UserRepository,
};
use chrono::Utc;
use tracing::error;

use super::service::{ProfileSpec, offers_for, owned_credential};

pub struct ResizeCustomerCluster<'a, C, V, D, DP, CR, P, A, U> {
    catalog: &'a C,
    provisioner: &'a V,
    deployments: D,
    data_planes: DP,
    credentials: CR,
    policy: P,
    audit: A,
    users: U,
}

impl<'a, C, V, D, DP, CR, P, A, U> ResizeCustomerCluster<'a, C, V, D, DP, CR, P, A, U>
where
    C: ProviderCatalog,
    V: ClusterProvisioner,
    D: DeploymentRepository,
    DP: DataPlaneRepository,
    CR: CloudCredentialRepository,
    P: DeploymentPolicy,
    A: AuditService,
    U: UserRepository,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        catalog: &'a C,
        provisioner: &'a V,
        deployments: D,
        data_planes: DP,
        credentials: CR,
        policy: P,
        audit: A,
        users: U,
    ) -> Self {
        Self {
            catalog,
            provisioner,
            deployments,
            data_planes,
            credentials,
            policy,
            audit,
            users,
        }
    }

    pub async fn execute(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
        spec: ProfileSpec,
    ) -> Result<ClusterProfile, CoreError> {
        self.policy
            .can_manage_deployments(identity.clone(), organisation_id)
            .await?;
        let actor = audit_actor(&identity, &self.users).await?;

        let mut deployment = self
            .deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .ok_or_else(|| not_found(deployment_id))?;
        let Distribution::CustomerCloud {
            credential_id,
            profile: current,
        } = deployment.distribution.clone()
        else {
            return Err(not_found(deployment_id));
        };

        let plane = self.settled_plane(&deployment).await?;
        let credential =
            owned_credential(&self.credentials, &credential_id, organisation_id).await?;
        let offers = offers_for(self.catalog, &credential, &plane.region).await?;

        let target = spec.into_profile(&offers)?;
        check_resize(&current, &target, &offers)?;

        self.provisioner.resize(&plane.id, &target).await?;

        deployment.distribution = Distribution::CustomerCloud {
            credential_id,
            profile: target.clone(),
        };
        deployment.updated_at = Utc::now();

        if let Err(cause) = self.record(&deployment, &current, &target, actor).await {
            error!(
                deployment = %deployment_id,
                %cause,
                "the provider resized the cluster but the record of it was not updated"
            );
            return Err(CoreError::ClusterResizeNotRecorded {
                deployment: deployment_id.0,
            });
        }

        Ok(target)
    }

    async fn settled_plane(&self, deployment: &Deployment) -> Result<DataPlane, CoreError> {
        let state = match deployment.status {
            DeploymentStatus::Deleting => Some("being deleted".to_string()),
            DeploymentStatus::Deleted => Some("deleted".to_string()),
            _ => None,
        };
        if let Some(state) = state {
            return Err(CoreError::ClusterNotReady {
                deployment: deployment.id.0,
                state,
            });
        }

        let plane = self
            .data_planes
            .find_by_id(&deployment.dataplane_id)
            .await?
            .ok_or(CoreError::DataPlaneNotFound {
                id: deployment.dataplane_id,
            })?;

        if plane.status != DataPlaneStatus::Active {
            return Err(CoreError::ClusterNotReady {
                deployment: deployment.id.0,
                state: plane.status.to_string(),
            });
        }

        Ok(plane)
    }

    async fn record(
        &self,
        deployment: &Deployment,
        before: &ClusterProfile,
        after: &ClusterProfile,
        actor: AuditActor,
    ) -> Result<(), CoreError> {
        self.deployments.update(deployment.clone()).await?;

        let change = AuditChange::new(profile_json(before)?, profile_json(after)?)?;
        self.audit
            .record(
                RecordAuditEntryCommand::new(
                    deployment.organisation_id,
                    actor,
                    AuditAction("deployment.cluster_profile.updated".to_string()),
                    AuditTarget {
                        kind: AuditTargetKind::Deployment,
                        id: deployment.id.0,
                    },
                )
                .with_change(change),
            )
            .await?;

        Ok(())
    }
}

fn not_found(deployment_id: DeploymentId) -> CoreError {
    CoreError::DeploymentNotFound {
        id: deployment_id.0,
    }
}

fn profile_json(profile: &ClusterProfile) -> Result<serde_json::Value, CoreError> {
    serde_json::to_value(profile).map_err(|error| CoreError::InternalError(error.to_string()))
}

fn check_resize(
    current: &ClusterProfile,
    target: &ClusterProfile,
    offers: &ProviderOffers,
) -> Result<(), ResizeError> {
    if current.node_type() != target.node_type() {
        return Err(ResizeError::NodeTypeChange {
            current: current.node_type().as_str().to_string(),
            requested: target.node_type().as_str().to_string(),
        });
    }

    let in_use = current.replication().get();
    if target.replication().get() < in_use {
        return Err(ResizeError::ReplicasExceedTarget {
            target: target.mode(),
            in_use,
            allowed: target.replication().get(),
        });
    }

    if current.mode() != target.mode() {
        current.resized_to(target.mode(), in_use, offers)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use autharie_domain::{
        audit::{AuditBatch, AuditEntry, AuditEntryId, commands::ListAuditEntriesCommand},
        dataplane::{
            cloud_provider::{
                CatalogError, ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money,
                NodeOffer, NodeType, Provider,
            },
            cluster_profile::{ClusterMode, Replication},
            credential::{CloudCredential, CloudCredentialId, ScopeCheck},
            provisioner::{ProvisionRequest, ProvisionedCluster},
            value_objects::{
                Capacity, DataPlaneAllocation, DataPlaneId, DataPlaneMode, DeploymentResources,
                PlacementRequest, Region,
            },
        },
        deployments::{DeploymentKind, DeploymentName},
        user::{User, UserId},
        version::Version,
    };
    use chrono::DateTime;
    use uuid::Uuid;

    use super::*;

    type Journal = Arc<Mutex<Vec<&'static str>>>;

    struct Policy {
        allowed: bool,
    }

    impl DeploymentPolicy for Policy {
        async fn can_view_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }

        async fn can_create_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }

        async fn can_manage_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            if self.allowed {
                Ok(())
            } else {
                Err(CoreError::PermissionDenied {
                    reason: "no".to_string(),
                })
            }
        }

        async fn can_delete_deployments(
            &self,
            _: Identity,
            _: OrganisationId,
        ) -> Result<(), CoreError> {
            Ok(())
        }
    }

    struct Catalog(ProviderOffers);

    impl ProviderCatalog for Catalog {
        async fn offers(
            &self,
            _provider: autharie_domain::dataplane::cloud_provider::Provider,
            _credential_id: &CloudCredentialId,
            _region: &Region,
        ) -> Result<ProviderOffers, CatalogError> {
            Ok(self.0.clone())
        }
    }

    struct Provisioner {
        journal: Journal,
        resizes: Mutex<Vec<ClusterProfile>>,
        fail: bool,
    }

    impl ClusterProvisioner for Provisioner {
        async fn provision(&self, _: ProvisionRequest) -> Result<ProvisionedCluster, CoreError> {
            unreachable!("a resize never provisions")
        }

        async fn deprovision(&self, _: &DataPlaneId) -> Result<(), CoreError> {
            unreachable!("a resize never deprovisions")
        }

        async fn resize(&self, _: &DataPlaneId, profile: &ClusterProfile) -> Result<(), CoreError> {
            self.journal.lock().expect("lock").push("resize");
            self.resizes.lock().expect("lock").push(profile.clone());
            if self.fail {
                return Err(CoreError::InternalError("provider down".to_string()));
            }
            Ok(())
        }
    }

    struct Deployments {
        journal: Journal,
        row: Mutex<Deployment>,
        fail_update: Mutex<bool>,
    }

    impl DeploymentRepository for &Deployments {
        async fn purge_deleted(&self, _: DateTime<Utc>) -> Result<u64, CoreError> {
            unreachable!()
        }

        async fn insert(&self, _: Deployment) -> Result<(), CoreError> {
            unreachable!()
        }

        async fn get_by_id(&self, _: DeploymentId) -> Result<Option<Deployment>, CoreError> {
            Ok(Some(self.row.lock().expect("lock").clone()))
        }

        async fn list_by_organisation(
            &self,
            _: OrganisationId,
        ) -> Result<Vec<Deployment>, CoreError> {
            unreachable!()
        }

        async fn update(&self, deployment: Deployment) -> Result<(), CoreError> {
            if *self.fail_update.lock().expect("lock") {
                return Err(CoreError::DatabaseError {
                    message: "down".to_string(),
                });
            }
            self.journal.lock().expect("lock").push("update");
            *self.row.lock().expect("lock") = deployment;
            Ok(())
        }

        async fn delete(&self, _: DeploymentId) -> Result<(), CoreError> {
            unreachable!()
        }

        async fn list_by_dataplane(&self, _: &DataPlaneId) -> Result<Vec<Deployment>, CoreError> {
            unreachable!()
        }

        async fn list_all_live(&self) -> Result<Vec<Deployment>, CoreError> {
            unreachable!()
        }

        async fn count_by_version(
            &self,
            _: &DeploymentKind,
        ) -> Result<Vec<(Version, u64)>, CoreError> {
            unreachable!()
        }
    }

    struct Planes(DataPlane);

    impl DataPlaneRepository for &Planes {
        async fn find_by_herald_subject(&self, _: &str) -> Result<Option<DataPlane>, CoreError> {
            unreachable!()
        }

        async fn find_by_id(&self, _: &DataPlaneId) -> Result<Option<DataPlane>, CoreError> {
            Ok(Some(self.0.clone()))
        }

        async fn find_active_shared_by_region(
            &self,
            _: &Region,
        ) -> Result<Vec<DataPlane>, CoreError> {
            unreachable!()
        }

        async fn find_available(
            &self,
            _: PlacementRequest,
        ) -> Result<Option<DataPlane>, CoreError> {
            unreachable!()
        }

        async fn region_is_served(&self, _: &Region) -> Result<bool, CoreError> {
            unreachable!()
        }

        async fn region_blocked_by_deployment_count(
            &self,
            _: &Region,
            _: DataPlaneMode,
            _: DeploymentResources,
        ) -> Result<bool, CoreError> {
            unreachable!()
        }

        async fn find_dedicated_for_organisation(
            &self,
            _: &OrganisationId,
            _: &Region,
        ) -> Result<Option<DataPlane>, CoreError> {
            unreachable!()
        }

        async fn list_all(&self) -> Result<Vec<DataPlane>, CoreError> {
            unreachable!()
        }

        async fn current_load(&self, _: &DataPlaneId) -> Result<u32, CoreError> {
            unreachable!()
        }

        async fn save(&self, _: &DataPlane) -> Result<(), CoreError> {
            unreachable!()
        }

        async fn touch_last_seen(
            &self,
            _: &DataPlaneId,
            _: DateTime<Utc>,
            _: Option<Version>,
            _: Option<String>,
        ) -> Result<bool, CoreError> {
            unreachable!()
        }
    }

    struct Credentials(CloudCredential);

    impl CloudCredentialRepository for &Credentials {
        async fn insert(&self, _: &CloudCredential) -> Result<(), CoreError> {
            unreachable!()
        }

        async fn get(&self, _: &CloudCredentialId) -> Result<Option<CloudCredential>, CoreError> {
            Ok(Some(self.0.clone()))
        }

        async fn list_for_organisation(
            &self,
            _: &OrganisationId,
        ) -> Result<Vec<CloudCredential>, CoreError> {
            unreachable!()
        }

        async fn is_in_use(&self, _: &CloudCredentialId) -> Result<bool, CoreError> {
            unreachable!()
        }
    }

    struct Audit {
        journal: Journal,
        entries: Mutex<Vec<RecordAuditEntryCommand>>,
    }

    impl AuditService for &Audit {
        async fn record(&self, command: RecordAuditEntryCommand) -> Result<AuditEntry, CoreError> {
            self.journal.lock().expect("lock").push("audit");
            self.entries.lock().expect("lock").push(command.clone());
            Ok(AuditEntry::record(
                AuditEntryId(Uuid::new_v4()),
                command.organisation_id,
                command.actor,
                command.action,
                command.target,
                command.change,
                Utc::now(),
            ))
        }

        async fn list_entries(
            &self,
            _: Identity,
            _: ListAuditEntriesCommand,
        ) -> Result<AuditBatch, CoreError> {
            unreachable!()
        }
    }

    struct Users;

    impl UserRepository for &Users {
        async fn upsert_by_email(&self, _: &User) -> Result<User, CoreError> {
            unreachable!()
        }

        async fn find_by_sub(&self, sub: &str) -> Result<Option<User>, CoreError> {
            Ok(Some(User {
                id: UserId(Uuid::new_v4()),
                email: "a@b.c".to_string(),
                name: "a".to_string(),
                sub: sub.to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }))
        }

        async fn find_by_email(&self, _: &str) -> Result<Option<User>, CoreError> {
            unreachable!()
        }
    }

    fn identity() -> Identity {
        Identity::Client(autharie_auth::Client {
            id: "c".to_string(),
            client_id: "client".to_string(),
            roles: vec![],
            scopes: vec![],
        })
    }

    fn mutualized() -> ControlPlaneOffer {
        ControlPlaneOffer {
            id: ControlPlaneOfferId::new("mutualized"),
            kind: ControlPlaneKind::Mutualized,
            monthly_price: Money::ZERO,
        }
    }

    fn offers() -> ProviderOffers {
        ProviderOffers {
            control_planes: vec![mutualized()],
            node_types: vec![
                NodeOffer {
                    node_type: NodeType::new("small"),
                    monthly_price: Money::new(1_000),
                },
                NodeOffer {
                    node_type: NodeType::new("large"),
                    monthly_price: Money::new(2_000),
                },
            ],
        }
    }

    fn profile(
        mode: ClusterMode,
        nodes: (u8, u8),
        replicas: u8,
        node_type: &str,
    ) -> ClusterProfile {
        ClusterProfile::new(
            mode,
            mutualized(),
            NodeType::new(node_type),
            nodes.0,
            nodes.1,
            Replication::new(replicas).expect("replicas"),
            &offers(),
        )
        .expect("valid profile")
    }

    fn spec(mode: ClusterMode, nodes: (u8, u8), replicas: u8, node_type: &str) -> ProfileSpec {
        ProfileSpec {
            mode,
            control_plane_id: ControlPlaneOfferId::new("mutualized"),
            node_type: NodeType::new(node_type),
            min_nodes: nodes.0,
            max_nodes: nodes.1,
            replication: Replication::new(replicas).expect("replicas"),
        }
    }

    struct World {
        journal: Journal,
        organisation: OrganisationId,
        deployment_id: DeploymentId,
        catalog: Catalog,
        provisioner: Provisioner,
        deployments: Deployments,
        planes: Planes,
        credentials: Credentials,
        audit: Audit,
        users: Users,
        allowed: bool,
    }

    impl World {
        fn new(current: ClusterProfile) -> Self {
            let journal = Journal::default();
            let organisation = OrganisationId(Uuid::new_v4());
            let credential_id = CloudCredentialId(Uuid::new_v4());
            let deployment_id = DeploymentId(Uuid::new_v4());
            let mut data_plane = DataPlane::new(
                DataPlaneAllocation::Customer {
                    organisation_id: organisation,
                    deployment_id,
                    credential_id,
                },
                Region::new("fr-par"),
                Capacity::new(1, 1, 1).expect("capacity"),
            );
            data_plane.status = DataPlaneStatus::Active;
            let at = Utc::now();
            let deployment = Deployment {
                id: deployment_id,
                organisation_id: organisation,
                dataplane_id: data_plane.id,
                name: DeploymentName("auth".to_string()),
                kind: DeploymentKind::Ferriskey,
                version: Version::new(26, 0, 1),
                status: DeploymentStatus::Successful,
                namespace: "production-auth".to_string(),
                environment: autharie_domain::deployments::environment::Environment::Development,
                offer: None,
                restored_from: None,
                resources: DeploymentResources::DEFAULT,
                created_by: UserId(Uuid::new_v4()),
                created_at: at,
                updated_at: at,
                deployed_at: None,
                deleted_at: None,
                auto_upgrade: Default::default(),
                maintenance_window: None,
                network_access: autharie_domain::deployments::network::NetworkAccess::Open,
                last_verified_restore_at: None,
                last_restore_drill_seconds: None,
                log_shipping_enabled: false,
                iam_settings: Default::default(),
                distribution: Distribution::CustomerCloud {
                    credential_id,
                    profile: current,
                },
            };

            Self {
                organisation,
                deployment_id,
                catalog: Catalog(offers()),
                provisioner: Provisioner {
                    journal: journal.clone(),
                    resizes: Mutex::new(Vec::new()),
                    fail: false,
                },
                deployments: Deployments {
                    journal: journal.clone(),
                    row: Mutex::new(deployment),
                    fail_update: Mutex::new(false),
                },
                planes: Planes(data_plane),
                credentials: Credentials(CloudCredential {
                    id: credential_id,
                    organisation_id: organisation,
                    provider: Provider::Scaleway,
                    label: "prod".to_string(),
                    scope_check: ScopeCheck { checked_at: at },
                    created_at: at,
                }),
                audit: Audit {
                    journal: journal.clone(),
                    entries: Mutex::new(Vec::new()),
                },
                users: Users,
                allowed: true,
                journal,
            }
        }

        fn status(self, status: DeploymentStatus) -> Self {
            self.deployments.row.lock().expect("lock").status = status;
            self
        }

        fn plane(mut self, status: DataPlaneStatus) -> Self {
            self.planes.0.status = status;
            self
        }

        fn distribution(self, distribution: Distribution) -> Self {
            self.deployments.row.lock().expect("lock").distribution = distribution;
            self
        }

        fn denying(mut self) -> Self {
            self.allowed = false;
            self
        }

        fn provider_down(mut self) -> Self {
            self.provisioner.fail = true;
            self
        }

        fn record_down(self, down: bool) -> Self {
            *self.deployments.fail_update.lock().expect("lock") = down;
            self
        }

        async fn resize(&self, spec: ProfileSpec) -> Result<ClusterProfile, CoreError> {
            self.resize_as(self.organisation, spec).await
        }

        async fn resize_as(
            &self,
            organisation: OrganisationId,
            spec: ProfileSpec,
        ) -> Result<ClusterProfile, CoreError> {
            ResizeCustomerCluster::new(
                &self.catalog,
                &self.provisioner,
                &self.deployments,
                &self.planes,
                &self.credentials,
                Policy {
                    allowed: self.allowed,
                },
                &self.audit,
                &self.users,
            )
            .execute(identity(), organisation, self.deployment_id, spec)
            .await
        }

        fn journal(&self) -> Vec<&'static str> {
            self.journal.lock().expect("lock").clone()
        }

        fn stored(&self) -> ClusterProfile {
            match &self.deployments.row.lock().expect("lock").distribution {
                Distribution::CustomerCloud { profile, .. } => profile.clone(),
                other => panic!("not a customer cloud: {other:?}"),
            }
        }
    }

    fn dev() -> ClusterProfile {
        profile(ClusterMode::Dev, (1, 1), 1, "small")
    }

    fn ha() -> ClusterProfile {
        profile(ClusterMode::Ha, (3, 5), 2, "small")
    }

    #[tokio::test]
    async fn a_caller_who_may_not_manage_deployments_is_refused_before_anything_moves() {
        let world = World::new(dev()).denying();

        let result = world
            .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
            .await;

        assert!(matches!(result, Err(CoreError::PermissionDenied { .. })));
        assert!(world.journal().is_empty());
    }

    #[tokio::test]
    async fn a_deployment_that_is_not_on_the_customer_cloud_is_not_found() {
        let world = World::new(dev()).distribution(Distribution::Shared);

        let result = world
            .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
            .await;

        assert!(matches!(result, Err(CoreError::DeploymentNotFound { .. })));
        assert!(world.journal().is_empty());
    }

    #[tokio::test]
    async fn a_deployment_of_another_organisation_is_not_found() {
        let world = World::new(dev());

        let result = world
            .resize_as(
                OrganisationId(Uuid::new_v4()),
                spec(ClusterMode::Standard, (2, 4), 2, "small"),
            )
            .await;

        assert!(matches!(result, Err(CoreError::DeploymentNotFound { .. })));
        assert!(world.journal().is_empty());
    }

    #[tokio::test]
    async fn a_plane_that_is_not_active_is_refused_and_the_provider_is_not_called() {
        for status in [DataPlaneStatus::Provisioning, DataPlaneStatus::Failed] {
            let world = World::new(dev()).plane(status);

            let result = world
                .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
                .await;

            assert!(matches!(result, Err(CoreError::ClusterNotReady { .. })));
            assert!(world.journal().is_empty());
        }
    }

    #[tokio::test]
    async fn a_deployment_being_deleted_is_refused_and_the_provider_is_not_called() {
        for status in [DeploymentStatus::Deleting, DeploymentStatus::Deleted] {
            let world = World::new(dev()).status(status);

            let result = world
                .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
                .await;

            assert!(matches!(result, Err(CoreError::ClusterNotReady { .. })));
            assert!(world.journal().is_empty());
        }
    }

    #[tokio::test]
    async fn another_node_type_is_refused_and_the_provider_is_not_called() {
        let world = World::new(dev());

        let result = world
            .resize(spec(ClusterMode::Standard, (2, 4), 2, "large"))
            .await;

        assert!(matches!(
            result,
            Err(CoreError::Resize(ResizeError::NodeTypeChange { .. }))
        ));
        assert!(world.journal().is_empty());
        assert_eq!(world.stored(), dev());
    }

    #[tokio::test]
    async fn replicas_below_the_ones_in_use_are_refused_and_the_provider_is_not_called() {
        let world = World::new(ha());

        let result = world
            .resize(spec(ClusterMode::Dev, (1, 1), 1, "small"))
            .await;

        assert!(matches!(
            result,
            Err(CoreError::Resize(ResizeError::ReplicasExceedTarget {
                in_use: 2,
                allowed: 1,
                ..
            }))
        ));
        assert!(world.journal().is_empty());
        assert_eq!(world.stored(), ha());
    }

    #[tokio::test]
    async fn an_invalid_target_profile_is_refused_before_the_provider() {
        let world = World::new(dev());

        let result = world
            .resize(spec(ClusterMode::Standard, (1, 4), 1, "small"))
            .await;

        assert!(matches!(result, Err(CoreError::Profile(_))));
        assert!(world.journal().is_empty());
    }

    #[tokio::test]
    async fn a_resize_calls_the_provider_once_then_records_it_with_an_audit_entry() {
        let world = World::new(dev());
        let target = spec(ClusterMode::Standard, (2, 4), 2, "small");

        let resized = world.resize(target).await.expect("resized");

        assert_eq!(resized.mode(), ClusterMode::Standard);
        assert_eq!(world.journal(), vec!["resize", "update", "audit"]);
        assert_eq!(world.provisioner.resizes.lock().expect("lock").len(), 1);
        assert_eq!(world.stored(), resized);
        let entries = world.audit.entries.lock().expect("lock");
        assert_eq!(entries[0].action.0, "deployment.cluster_profile.updated");
    }

    #[tokio::test]
    async fn a_provider_failure_leaves_the_record_unchanged() {
        let world = World::new(dev()).provider_down();

        let result = world
            .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
            .await;

        assert!(matches!(result, Err(CoreError::InternalError(_))));
        assert_eq!(world.journal(), vec!["resize"]);
        assert_eq!(world.stored(), dev());
    }

    #[tokio::test]
    async fn a_record_failure_after_the_provider_succeeded_says_so() {
        let world = World::new(dev()).record_down(true);

        let result = world
            .resize(spec(ClusterMode::Standard, (2, 4), 2, "small"))
            .await;

        assert!(matches!(
            result,
            Err(CoreError::ClusterResizeNotRecorded { .. })
        ));
        assert_eq!(world.journal(), vec!["resize"]);
        assert_eq!(world.stored(), dev());
    }

    #[tokio::test]
    async fn repeating_the_request_after_a_record_failure_brings_the_record_in_line() {
        let world = World::new(dev()).record_down(true);
        let target = || spec(ClusterMode::Standard, (2, 4), 2, "small");

        assert!(world.resize(target()).await.is_err());
        *world.deployments.fail_update.lock().expect("lock") = false;
        let resized = world.resize(target()).await.expect("resized");

        assert_eq!(world.stored(), resized);
        assert_eq!(world.provisioner.resizes.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn the_same_target_twice_succeeds_both_times() {
        let world = World::new(dev());
        let target = || spec(ClusterMode::Standard, (2, 4), 2, "small");

        let first = world.resize(target()).await.expect("first");
        let second = world.resize(target()).await.expect("second");

        assert_eq!(first, second);
        assert_eq!(world.stored(), second);
    }
}
