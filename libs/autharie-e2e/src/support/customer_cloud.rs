use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use autharie_auth::Identity;
use autharie_core::{
    CloudProviders, EnvelopeCredentialStore, FixedCloudProvider, FixedVerdict, HelmBootstrapper,
    HelmConfig, HelmOutcome, HelmRunner, ScalewayConfig, ScalewayProvisioner,
    customer_clusters::WorkerSettings,
    customer_clusters::{ClaimedCluster, CustomerClusterQueue, CustomerClusterWorker},
};
use autharie_domain::{
    CoreError,
    audit::{
        AuditBatch, AuditEntry, AuditEntryId,
        commands::{ListAuditEntriesCommand, RecordAuditEntryCommand},
        ports::AuditService,
    },
    backups::{
        keys::{DataKey, Dek, KeyError, KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
        ports::{KeyProvider, KeyProviderAdmin},
    },
    dataplane::{
        cloud_provider::{
            ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer, NodeType,
            ProviderOffers,
        },
        cluster_profile::{ClusterMode, ClusterProfile, Replication},
        credential::{CloudCredential, CloudCredentialId},
        credential_repository::{
            CloudCredentialRepository, SealedCredential, SealedSecretRepository,
        },
        entities::DataPlane,
        herald_identity::{HeraldBinding, MintedHeraldIdentity},
        inventory::{ClusterInventory, ProvisionedResource},
        ports::{DataPlaneRepository, HeraldIdentityProvisioner},
        provisioner::{ClusterProvisioner, ProvisionRequest, ProvisionedCluster},
        value_objects::{
            DataPlaneAllocation, DataPlaneId, DataPlaneMode, DataPlaneStatus, DeploymentResources,
            PlacementRequest, Region,
        },
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind,
        distribution::Distribution,
        ports::{DeploymentPolicy, DeploymentRepository},
    },
    organisation::OrganisationId,
    role::ports::PermissionProvider,
    user::{User, UserId, ports::UserRepository},
    version::Version,
};
use autharie_permission::Permissions;
use chrono::{DateTime, Utc};
use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, Targets},
    fmt::MakeWriter,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};
use uuid::Uuid;

pub const REGION: &str = "fr-par";
pub const MUTUALIZED: &str = "kapsule";
pub const DEDICATED: &str = "kapsule-dedicated-4";
pub const SMALL: &str = "PRO2-S";
pub const MEDIUM: &str = "PRO2-M";
pub const MUTUALIZED_PRICE: u64 = 0;
pub const DEDICATED_PRICE: u64 = 10_800;
pub const SMALL_PRICE: u64 = 7_200;
pub const MEDIUM_PRICE: u64 = 14_400;

pub fn control_plane(id: &str, kind: ControlPlaneKind, price: u64) -> ControlPlaneOffer {
    ControlPlaneOffer {
        id: ControlPlaneOfferId::new(id),
        kind,
        monthly_price: Money::new(price),
    }
}

pub fn catalog() -> ProviderOffers {
    ProviderOffers {
        control_planes: vec![
            control_plane(MUTUALIZED, ControlPlaneKind::Mutualized, MUTUALIZED_PRICE),
            control_plane(DEDICATED, ControlPlaneKind::Dedicated, DEDICATED_PRICE),
        ],
        node_types: vec![
            NodeOffer {
                node_type: NodeType::new(SMALL),
                monthly_price: Money::new(SMALL_PRICE),
            },
            NodeOffer {
                node_type: NodeType::new(MEDIUM),
                monthly_price: Money::new(MEDIUM_PRICE),
            },
        ],
    }
}

pub fn providers(verdict: FixedVerdict) -> CloudProviders {
    CloudProviders::Fixed(FixedCloudProvider {
        offers: catalog(),
        verdict,
    })
}

pub fn profile(
    mode: ClusterMode,
    control_plane_id: &str,
    node_type: &str,
    min: u8,
    max: u8,
    replicas: u8,
) -> Result<ClusterProfile, autharie_domain::dataplane::cluster_profile::ProfileError> {
    let offers = catalog();
    let offer = offers
        .control_plane(&ControlPlaneOfferId::new(control_plane_id))
        .cloned()
        .unwrap_or_else(|| control_plane(control_plane_id, ControlPlaneKind::Mutualized, 0));
    ClusterProfile::new(
        mode,
        offer,
        NodeType::new(node_type),
        min,
        max,
        Replication::new(replicas).expect("at least one replica"),
        &offers,
    )
}

pub fn credential_json(secret_key: &str) -> String {
    format!(r#"{{"access_key":"SCWACCESSKEY","secret_key":"{secret_key}","project_id":"proj-1"}}"#)
}

#[derive(Debug, Default)]
struct State {
    deployments: Vec<Deployment>,
    planes: Vec<DataPlane>,
    inventory: Vec<(DataPlaneId, ProvisionedResource, bool)>,
    credentials: Vec<CloudCredential>,
    sealed: Vec<SealedCredential>,
    claimed: Vec<DataPlaneId>,
}

#[derive(Debug, Clone, Default)]
pub struct Platform {
    state: Arc<Mutex<State>>,
}

impl Platform {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("not poisoned")
    }

    pub fn deployments(&self) -> Vec<Deployment> {
        self.state().deployments.clone()
    }

    pub fn deployment(&self, id: DeploymentId) -> Option<Deployment> {
        self.state()
            .deployments
            .iter()
            .find(|deployment| deployment.id == id)
            .cloned()
    }

    pub fn planes(&self) -> Vec<DataPlane> {
        self.state().planes.clone()
    }

    pub fn plane(&self, id: DataPlaneId) -> Option<DataPlane> {
        self.state()
            .planes
            .iter()
            .find(|plane| plane.id == id)
            .cloned()
    }

    pub fn inventory(&self) -> Vec<(ProvisionedResource, bool)> {
        self.state()
            .inventory
            .iter()
            .map(|(_, resource, released)| (resource.clone(), *released))
            .collect()
    }

    pub fn sealed(&self) -> Vec<SealedCredential> {
        self.state().sealed.clone()
    }

    pub fn credentials(&self) -> Vec<CloudCredential> {
        self.state().credentials.clone()
    }

    pub fn dump(&self) -> String {
        format!("{:?}", *self.state())
    }
}

impl DeploymentRepository for Platform {
    async fn purge_deleted(&self, _: DateTime<Utc>) -> Result<u64, CoreError> {
        Ok(0)
    }

    async fn insert(&self, deployment: Deployment) -> Result<(), CoreError> {
        self.state().deployments.push(deployment);
        Ok(())
    }

    async fn get_by_id(
        &self,
        deployment_id: DeploymentId,
    ) -> Result<Option<Deployment>, CoreError> {
        Ok(self.deployment(deployment_id))
    }

    async fn list_by_organisation(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Deployment>, CoreError> {
        Ok(self
            .state()
            .deployments
            .iter()
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .cloned()
            .collect())
    }

    async fn update(&self, deployment: Deployment) -> Result<(), CoreError> {
        let mut state = self.state();
        if let Some(row) = state
            .deployments
            .iter_mut()
            .find(|row| row.id == deployment.id)
        {
            *row = deployment;
        }
        Ok(())
    }

    async fn delete(&self, deployment_id: DeploymentId) -> Result<(), CoreError> {
        let mut state = self.state();
        if let Some(row) = state
            .deployments
            .iter_mut()
            .find(|row| row.id == deployment_id)
        {
            let at = Utc::now();
            row.deleted_at = Some(at);
            row.updated_at = at;
            row.status = autharie_domain::deployments::DeploymentStatus::Deleting;
        }
        Ok(())
    }

    async fn list_by_dataplane(
        &self,
        dataplane_id: &DataPlaneId,
    ) -> Result<Vec<Deployment>, CoreError> {
        Ok(self
            .state()
            .deployments
            .iter()
            .filter(|deployment| &deployment.dataplane_id == dataplane_id)
            .cloned()
            .collect())
    }

    async fn list_all_live(&self) -> Result<Vec<Deployment>, CoreError> {
        Ok(Vec::new())
    }

    async fn count_by_version(&self, _: &DeploymentKind) -> Result<Vec<(Version, u64)>, CoreError> {
        Ok(Vec::new())
    }
}

impl DataPlaneRepository for Platform {
    async fn find_by_herald_subject(&self, subject: &str) -> Result<Option<DataPlane>, CoreError> {
        Ok(self
            .state()
            .planes
            .iter()
            .find(|plane| {
                plane
                    .herald
                    .as_ref()
                    .is_some_and(|binding| binding.subject == subject)
            })
            .cloned())
    }

    async fn find_by_id(&self, id: &DataPlaneId) -> Result<Option<DataPlane>, CoreError> {
        Ok(self.plane(*id))
    }

    async fn find_active_shared_by_region(&self, _: &Region) -> Result<Vec<DataPlane>, CoreError> {
        Ok(Vec::new())
    }

    async fn find_available(&self, _: PlacementRequest) -> Result<Option<DataPlane>, CoreError> {
        Ok(None)
    }

    async fn region_is_served(&self, _: &Region) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn region_blocked_by_deployment_count(
        &self,
        _: &Region,
        _: DataPlaneMode,
        _: DeploymentResources,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn find_dedicated_for_organisation(
        &self,
        _: &OrganisationId,
        _: &Region,
    ) -> Result<Option<DataPlane>, CoreError> {
        Ok(None)
    }

    async fn list_all(&self) -> Result<Vec<DataPlane>, CoreError> {
        Ok(self.planes())
    }

    async fn current_load(&self, _: &DataPlaneId) -> Result<u32, CoreError> {
        Ok(0)
    }

    async fn save(&self, dataplane: &DataPlane) -> Result<(), CoreError> {
        let mut state = self.state();
        match state.planes.iter_mut().find(|row| row.id == dataplane.id) {
            Some(row) => *row = dataplane.clone(),
            None => state.planes.push(dataplane.clone()),
        }
        Ok(())
    }

    async fn touch_last_seen(
        &self,
        id: &DataPlaneId,
        at: DateTime<Utc>,
        operator_version: Option<Version>,
        gateway_address: Option<String>,
    ) -> Result<bool, CoreError> {
        let mut state = self.state();
        let Some(plane) = state.planes.iter_mut().find(|plane| &plane.id == id) else {
            return Ok(false);
        };
        plane.last_seen_at = Some(at);
        if plane.status == DataPlaneStatus::Provisioning {
            plane.status = DataPlaneStatus::Active;
        }
        if operator_version.is_some() {
            plane.operator_version = operator_version;
        }
        if gateway_address.is_some() {
            plane.gateway_address = gateway_address;
        }
        Ok(true)
    }
}

impl ClusterInventory for Platform {
    async fn record(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        let mut state = self.state();
        let known = state
            .inventory
            .iter()
            .any(|(owner, held, _)| owner == data_plane_id && held == resource);
        if !known {
            state
                .inventory
                .push((*data_plane_id, resource.clone(), false));
        }
        Ok(())
    }

    async fn unreleased(
        &self,
        data_plane_id: &DataPlaneId,
    ) -> Result<Vec<ProvisionedResource>, CoreError> {
        Ok(self
            .state()
            .inventory
            .iter()
            .filter(|(owner, _, released)| owner == data_plane_id && !released)
            .map(|(_, resource, _)| resource.clone())
            .collect())
    }

    async fn mark_released(
        &self,
        data_plane_id: &DataPlaneId,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        for (owner, held, released) in self.state().inventory.iter_mut() {
            if owner == data_plane_id && held == resource {
                *released = true;
            }
        }
        Ok(())
    }
}

impl CloudCredentialRepository for Platform {
    async fn insert(&self, credential: &CloudCredential) -> Result<(), CoreError> {
        self.state().credentials.push(credential.clone());
        Ok(())
    }

    async fn get(&self, id: &CloudCredentialId) -> Result<Option<CloudCredential>, CoreError> {
        Ok(self
            .state()
            .credentials
            .iter()
            .find(|row| &row.id == id)
            .cloned())
    }

    async fn list_for_organisation(
        &self,
        organisation_id: &OrganisationId,
    ) -> Result<Vec<CloudCredential>, CoreError> {
        Ok(self
            .state()
            .credentials
            .iter()
            .filter(|row| &row.organisation_id == organisation_id)
            .cloned()
            .collect())
    }

    async fn is_in_use(&self, id: &CloudCredentialId) -> Result<bool, CoreError> {
        let state = self.state();
        let deployed = state.deployments.iter().any(|deployment| {
            matches!(&deployment.distribution, Distribution::CustomerCloud { credential_id, .. } if credential_id == id)
                && deployment.status != autharie_domain::deployments::DeploymentStatus::Deleted
        });
        let holding = state.planes.iter().any(|plane| {
            matches!(plane.allocation, DataPlaneAllocation::Customer { credential_id, .. } if &credential_id == id)
                && state
                    .inventory
                    .iter()
                    .any(|(owner, _, released)| *owner == plane.id && !released)
        });
        Ok(deployed || holding)
    }
}

impl SealedSecretRepository for Platform {
    async fn insert(&self, credential: &SealedCredential) -> Result<(), CoreError> {
        self.state().sealed.push(credential.clone());
        Ok(())
    }

    async fn get(&self, id: &CloudCredentialId) -> Result<Option<SealedCredential>, CoreError> {
        Ok(self
            .state()
            .sealed
            .iter()
            .find(|row| &row.id == id)
            .cloned())
    }

    async fn delete(&self, id: &CloudCredentialId) -> Result<bool, CoreError> {
        let mut state = self.state();
        let before = state.sealed.len();
        state.sealed.retain(|row| &row.id != id);
        Ok(state.sealed.len() != before)
    }
}

impl CustomerClusterQueue for Platform {
    async fn claim(&self, _: Duration, limit: u32) -> Result<Vec<ClaimedCluster>, CoreError> {
        let mut state = self.state();
        let State {
            deployments,
            planes,
            claimed,
            ..
        } = &mut *state;
        let mut found = Vec::new();
        for plane in planes.iter() {
            if found.len() >= limit as usize {
                break;
            }
            let DataPlaneAllocation::Customer {
                deployment_id,
                credential_id,
                ..
            } = plane.allocation
            else {
                continue;
            };
            if plane.status != DataPlaneStatus::Provisioning
                || plane.herald.is_some()
                || claimed.contains(&plane.id)
            {
                continue;
            }
            let Some(deployment) = deployments.iter().find(|deployment| {
                deployment.id == deployment_id && deployment.deleted_at.is_none()
            }) else {
                continue;
            };
            let Distribution::CustomerCloud { profile, .. } = &deployment.distribution else {
                continue;
            };
            claimed.push(plane.id);
            found.push(ClaimedCluster {
                data_plane: plane.clone(),
                deployment_id,
                credential_id,
                profile: profile.clone(),
                resources: deployment.resources,
            });
        }
        Ok(found)
    }

    async fn complete(&self, data_plane: &DataPlane) -> Result<bool, CoreError> {
        let Some(herald) = data_plane.herald.clone() else {
            return Err(CoreError::InternalError(
                "completed without a herald binding".to_string(),
            ));
        };
        let mut state = self.state();
        state.claimed.retain(|id| *id != data_plane.id);
        let Some(stored) = state
            .planes
            .iter_mut()
            .find(|plane| plane.id == data_plane.id)
        else {
            return Ok(false);
        };
        if stored.status != DataPlaneStatus::Provisioning {
            return Ok(false);
        }
        stored.herald = Some(herald);
        stored.capacity = data_plane.capacity;
        Ok(true)
    }

    async fn fail(&self, id: &DataPlaneId, reason: &str) -> Result<bool, CoreError> {
        let mut state = self.state();
        state.claimed.retain(|claimed| claimed != id);
        let Some(stored) = state.planes.iter_mut().find(|plane| &plane.id == id) else {
            return Ok(false);
        };
        if stored.status != DataPlaneStatus::Provisioning {
            return Ok(false);
        }
        stored.fail(reason);
        Ok(true)
    }

    async fn teardown_candidates(&self, limit: u32) -> Result<Vec<DataPlane>, CoreError> {
        let state = self.state();
        Ok(state
            .planes
            .iter()
            .filter(|plane| {
                let DataPlaneAllocation::Customer { deployment_id, .. } = plane.allocation else {
                    return false;
                };
                if state.claimed.contains(&plane.id) {
                    return false;
                }
                let gone = state
                    .deployments
                    .iter()
                    .find(|deployment| deployment.id == deployment_id)
                    .is_none_or(|deployment| deployment.deleted_at.is_some());
                let holding = state
                    .inventory
                    .iter()
                    .any(|(owner, _, released)| *owner == plane.id && !released);
                let ended = matches!(
                    plane.status,
                    DataPlaneStatus::Disabled | DataPlaneStatus::Failed
                );
                (gone && !ended) || ((gone || plane.status == DataPlaneStatus::Failed) && holding)
            })
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn disable(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        let mut state = self.state();
        let Some(stored) = state.planes.iter_mut().find(|plane| &plane.id == id) else {
            return Ok(false);
        };
        if matches!(
            stored.status,
            DataPlaneStatus::Failed | DataPlaneStatus::Disabled
        ) {
            return Ok(false);
        }
        stored.disable();
        Ok(true)
    }

    async fn awaiting_deletion(&self, limit: u32) -> Result<Vec<DataPlaneId>, CoreError> {
        let state = self.state();
        Ok(state
            .planes
            .iter()
            .filter(|plane| {
                let DataPlaneAllocation::Customer { deployment_id, .. } = plane.allocation else {
                    return false;
                };
                let deleting = state.deployments.iter().any(|deployment| {
                    deployment.id == deployment_id
                        && deployment.status
                            == autharie_domain::deployments::DeploymentStatus::Deleting
                        && deployment.deleted_at.is_some()
                });
                let holding = state
                    .inventory
                    .iter()
                    .any(|(owner, _, released)| *owner == plane.id && !released);
                matches!(
                    plane.status,
                    DataPlaneStatus::Disabled | DataPlaneStatus::Failed
                ) && deleting
                    && !holding
                    && !state.claimed.contains(&plane.id)
            })
            .take(limit as usize)
            .map(|plane| plane.id)
            .collect())
    }

    async fn confirm_deleted(&self, id: &DataPlaneId) -> Result<bool, CoreError> {
        let mut state = self.state();
        let Some(DataPlaneAllocation::Customer { deployment_id, .. }) = state
            .planes
            .iter()
            .find(|plane| &plane.id == id)
            .map(|plane| plane.allocation)
        else {
            return Ok(false);
        };
        let Some(deployment) = state
            .deployments
            .iter_mut()
            .find(|deployment| deployment.id == deployment_id)
        else {
            return Ok(false);
        };
        Ok(deployment.confirm_deletion(Utc::now()))
    }
}

#[derive(Debug, Default)]
pub struct Keys {
    wrapped: Mutex<HashMap<String, Vec<u8>>>,
}

impl KeyProvider for Keys {
    async fn generate_data_key(&self, name: &KeyName) -> Result<DataKey, KeyError> {
        let mut key = Uuid::new_v4().as_bytes().to_vec();
        key.extend_from_slice(Uuid::new_v4().as_bytes());
        let wrapped = format!("wrapped:{}", Uuid::new_v4());
        self.wrapped
            .lock()
            .expect("not poisoned")
            .insert(wrapped.clone(), key.clone());
        Ok(DataKey::new(
            Dek::new(key),
            WrappedDek::new(wrapped),
            KeyRef::new(ProviderName::platform(), name.clone(), KeyVersion::new(1)),
        ))
    }

    async fn unwrap_data_key(&self, _: &KeyRef, wrapped: &WrappedDek) -> Result<Dek, KeyError> {
        self.wrapped
            .lock()
            .expect("not poisoned")
            .get(wrapped.as_str())
            .map(|bytes| Dek::new(bytes.clone()))
            .ok_or(KeyError::Refused {
                operation: "unwrap".to_string(),
                reason: "unknown data key".to_string(),
            })
    }
}

impl KeyProviderAdmin for Keys {
    async fn ensure_key(&self, _: &KeyName) -> Result<(), KeyError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct IdentityLog {
    minted: Vec<DataPlaneId>,
    revoked: Vec<DataPlaneId>,
    secrets: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Identities {
    log: Arc<Mutex<IdentityLog>>,
}

impl Identities {
    pub fn minted(&self) -> Vec<DataPlaneId> {
        self.log.lock().expect("not poisoned").minted.clone()
    }

    pub fn revoked(&self) -> Vec<DataPlaneId> {
        self.log.lock().expect("not poisoned").revoked.clone()
    }

    pub fn secrets(&self) -> Vec<String> {
        self.log.lock().expect("not poisoned").secrets.clone()
    }
}

impl HeraldIdentityProvisioner for Identities {
    async fn mint(&self, dataplane: DataPlaneId) -> Result<MintedHeraldIdentity, CoreError> {
        let secret = format!("herald-secret-{}", Uuid::new_v4().simple());
        let mut log = self.log.lock().expect("not poisoned");
        log.minted.push(dataplane);
        log.secrets.push(secret.clone());
        Ok(MintedHeraldIdentity {
            binding: HeraldBinding {
                client_id: MintedHeraldIdentity::client_id_for(dataplane),
                subject: format!("subject-{}", dataplane.0),
            },
            secret,
        })
    }

    async fn revoke(&self, dataplane: DataPlaneId) -> Result<(), CoreError> {
        self.log
            .lock()
            .expect("not poisoned")
            .revoked
            .push(dataplane);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct HelmCall {
    pub args: Vec<String>,
    pub dir: PathBuf,
    pub kubeconfig: String,
    pub values: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Runner {
    calls: Arc<Mutex<Vec<HelmCall>>>,
}

impl Runner {
    pub fn calls(&self) -> Vec<HelmCall> {
        self.calls.lock().expect("not poisoned").clone()
    }

    pub fn releases(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|call| call.args[2].clone())
            .collect()
    }
}

impl HelmRunner for Runner {
    async fn run(&self, args: &[String], working_dir: &Path) -> Result<HelmOutcome, CoreError> {
        let values = args
            .windows(2)
            .find(|pair| pair[0] == "--values")
            .and_then(|pair| std::fs::read_to_string(&pair[1]).ok());
        let kubeconfig =
            std::fs::read_to_string(working_dir.join("kubeconfig")).unwrap_or_default();
        self.calls.lock().expect("not poisoned").push(HelmCall {
            args: args.to_vec(),
            dir: working_dir.to_path_buf(),
            kubeconfig,
            values,
        });
        Ok(HelmOutcome {
            code: Some(0),
            last_line: String::new(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct NeverProvisions {
    calls: Arc<AtomicUsize>,
}

impl NeverProvisions {
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ClusterProvisioner for NeverProvisions {
    async fn provision(&self, _: ProvisionRequest) -> Result<ProvisionedCluster, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CoreError::InternalError(
            "the create request must not provision".to_string(),
        ))
    }

    async fn deprovision(&self, _: &DataPlaneId) -> Result<(), CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn resize(&self, _: &DataPlaneId, _: &ClusterProfile) -> Result<(), CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Audit {
    entries: Arc<Mutex<Vec<RecordAuditEntryCommand>>>,
}

impl Audit {
    pub fn entries(&self) -> Vec<RecordAuditEntryCommand> {
        self.entries.lock().expect("not poisoned").clone()
    }
}

impl AuditService for Audit {
    async fn record(&self, command: RecordAuditEntryCommand) -> Result<AuditEntry, CoreError> {
        self.entries
            .lock()
            .expect("not poisoned")
            .push(command.clone());
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
        Err(CoreError::InternalError(
            "this double only records".to_string(),
        ))
    }
}

pub fn member() -> Identity {
    Identity::User(autharie_auth::User {
        id: Uuid::from_u128(7).to_string(),
        username: "member".to_string(),
        email: None,
        name: None,
        roles: vec![],
    })
}

#[derive(Debug, Clone, Copy)]
pub struct Users;

impl UserRepository for Users {
    async fn upsert_by_email(&self, user: &User) -> Result<User, CoreError> {
        Ok(user.clone())
    }

    async fn find_by_sub(&self, sub: &str) -> Result<Option<User>, CoreError> {
        let Ok(id) = Uuid::parse_str(sub) else {
            return Ok(None);
        };
        Ok(Some(User {
            id: UserId(id),
            email: "customer@example.com".to_string(),
            name: "Customer".to_string(),
            sub: sub.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn find_by_email(&self, _: &str) -> Result<Option<User>, CoreError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AllowAll;

impl DeploymentPolicy for AllowAll {
    async fn can_view_deployments(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
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
        Ok(())
    }

    async fn can_delete_deployments(
        &self,
        _: Identity,
        _: OrganisationId,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Grants(pub Permissions);

impl Grants {
    pub fn administrator() -> Self {
        Self(Permissions::ADMINISTRATOR)
    }
}

impl PermissionProvider for Grants {
    async fn permissions_for_organisation(
        &self,
        _: Identity,
        _: OrganisationId,
    ) -> Result<Permissions, CoreError> {
        Ok(self.0)
    }
}

#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static CAPTURE: OnceLock<Capture> = OnceLock::new();

pub fn capture() -> &'static Capture {
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let targets = Targets::new()
            .with_default(LevelFilter::TRACE)
            .with_target("httpmock", LevelFilter::OFF);
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(capture.clone())
                    .with_ansi(false)
                    .with_filter(targets),
            )
            .try_init()
            .ok();
        capture
    })
}

pub type Provisioner<'a> = ScalewayProvisioner<
    EnvelopeCredentialStore<'a, Platform, Keys>,
    Platform,
    Platform,
    HelmBootstrapper<Identities, Runner>,
>;

pub type Worker<'a> = CustomerClusterWorker<Platform, Provisioner<'a>, Identities>;

pub fn scaleway_config(base_url: &str) -> ScalewayConfig {
    ScalewayConfig {
        base_url: base_url.to_string(),
        poll_interval: Duration::ZERO,
        poll_attempts: 3,
        ..ScalewayConfig::default()
    }
}

pub fn provisioner<'a>(
    base_url: &str,
    platform: &Platform,
    keys: &'a Keys,
    identities: &Identities,
    runner: &Runner,
) -> Provisioner<'a> {
    let credentials =
        EnvelopeCredentialStore::new(platform.clone(), keys).expect("a credential store");
    let bootstrapper = HelmBootstrapper::with_runner(
        identities.clone(),
        runner.clone(),
        HelmConfig::new("https://cp.example", "https://id.example/realms/autharie"),
    );
    ScalewayProvisioner::new(
        scaleway_config(base_url),
        credentials,
        platform.clone(),
        platform.clone(),
        bootstrapper,
    )
    .expect("a provisioner")
}

pub fn worker<'a>(
    base_url: &str,
    platform: &Platform,
    keys: &'a Keys,
    identities: &Identities,
    runner: &Runner,
) -> Worker<'a> {
    CustomerClusterWorker::new(
        platform.clone(),
        provisioner(base_url, platform, keys, identities, runner),
        identities.clone(),
        WorkerSettings::default(),
    )
}
