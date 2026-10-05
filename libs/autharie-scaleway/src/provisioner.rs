use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        cluster_profile::{ClusterProfile, ProfileError},
        credential::{CloudCredentialId, CloudCredentialStore, SecretString},
        herald_identity::HeraldBinding,
        inventory::{ClusterInventory, ProvisionedResource, ResourceKind},
        ports::DataPlaneRepository,
        provisioner::{
            ClusterProvisioner, ProvisionError, ProvisionRequest, ProvisionTarget,
            ProvisionedCluster,
        },
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, Region},
    },
    organisation::OrganisationId,
};
use serde_json::json;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::{
    config::ScalewayConfig,
    error::{ApiError, ScalewayError},
    http::{Http, Session},
    layout::Layout,
    models::{Cluster, ClusterTypeList, Pool, PrivateNetwork, ServerTypeList, VersionList},
};

const MIB: u64 = 1024 * 1024;
const STATUS_READY: &str = "ready";
const STATUS_DELETED: &str = "deleted";
const POOL_NAME: &str = "autharie-pool";

pub struct ScalewayProvisioner<S, I, R, B> {
    http: Http,
    config: ScalewayConfig,
    credentials: S,
    inventory: I,
    data_planes: R,
    bootstrapper: B,
}

impl<S, I, R, B> ScalewayProvisioner<S, I, R, B>
where
    S: CloudCredentialStore,
    I: ClusterInventory,
    R: DataPlaneRepository,
    B: ClusterBootstrapper,
{
    pub fn new(
        config: ScalewayConfig,
        credentials: S,
        inventory: I,
        data_planes: R,
        bootstrapper: B,
    ) -> Result<Self, ScalewayError> {
        Ok(Self {
            http: Http::new(&config)?,
            config,
            credentials,
            inventory,
            data_planes,
            bootstrapper,
        })
    }

    async fn open(&self, credential_id: &CloudCredentialId) -> Result<Session<'_>, CoreError> {
        let secret = self.credentials.get_for_provisioning(credential_id).await?;
        Ok(Session::open(&self.http, &secret)?)
    }

    async fn locate(&self, id: &DataPlaneId) -> Result<(CloudCredentialId, Region), CoreError> {
        let data_plane = self
            .data_planes
            .find_by_id(id)
            .await?
            .ok_or(CoreError::DataPlaneNotFound { id: *id })?;
        match data_plane.allocation {
            DataPlaneAllocation::Customer { credential_id, .. } => {
                Ok((credential_id, data_plane.region))
            }
            _ => Err(CoreError::InternalError(format!(
                "data plane {id} was not created in a customer account"
            ))),
        }
    }

    async fn capacity(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        profile: &ClusterProfile,
        storage_gib: u32,
    ) -> Result<Capacity, CoreError> {
        let servers: ServerTypeList = session
            .get(&layout.server_types(), &[])
            .await
            .map_err(|error| failure("read node types", error))?;
        let node = servers
            .servers
            .get(profile.node_type().as_str())
            .ok_or_else(|| ProfileError::NodeTypeUnavailable {
                node_type: profile.node_type().as_str().to_string(),
            })?;
        let nodes = u32::from(profile.min_nodes());
        let cpu_millis = node.ncpus.saturating_mul(1000).saturating_mul(nodes);
        let memory_mib = u32::try_from(node.ram / MIB)
            .unwrap_or(u32::MAX)
            .saturating_mul(nodes);
        Capacity::new(cpu_millis, memory_mib, storage_gib.max(1))
    }

    async fn record_created(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        data_plane_id: &DataPlaneId,
        resource: ProvisionedResource,
    ) -> Result<(), CoreError> {
        match self.inventory.record(data_plane_id, &resource).await {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Err(cleanup) = self.delete_resource(session, layout, &resource).await {
                    warn!(
                        kind = resource.kind.as_str(),
                        provider_id = %resource.provider_id,
                        error = %cleanup,
                        "a resource created but not recorded could not be deleted"
                    );
                }
                Err(error)
            }
        }
    }

    async fn build(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        id: &DataPlaneId,
        region: &Region,
        organisation_id: OrganisationId,
        profile: &ClusterProfile,
    ) -> Result<HeraldBinding, CoreError> {
        let name = format!("autharie-{id}");
        let tags = ["autharie".to_string(), format!("data-plane-{id}")];

        let network: PrivateNetwork = session
            .post(
                &layout.private_networks(),
                &json!({
                    "name": name,
                    "project_id": session.project_id,
                    "tags": tags,
                }),
            )
            .await
            .map_err(|error| failure("create the private network", error))?;
        self.record_created(
            session,
            layout,
            id,
            ProvisionedResource {
                kind: ResourceKind::PrivateNetwork,
                provider_id: network.id.clone(),
            },
        )
        .await?;
        info!(data_plane_id = %id, "private network created");

        let version = self.kubernetes_version(session, layout).await?;
        let cluster: Cluster = session
            .post(
                &layout.clusters(),
                &json!({
                    "name": name,
                    "type": profile.control_plane().id.as_str(),
                    "version": version,
                    "cni": self.config.cni,
                    "project_id": session.project_id,
                    "private_network_id": network.id,
                    "tags": tags,
                }),
            )
            .await
            .map_err(|error| failure("create the cluster", error))?;
        self.record_created(
            session,
            layout,
            id,
            ProvisionedResource {
                kind: ResourceKind::Cluster,
                provider_id: cluster.id.clone(),
            },
        )
        .await?;
        info!(data_plane_id = %id, "cluster created");

        let autoscaling = profile.mode().limits().autoscaling;
        let mut pool_body = json!({
            "name": POOL_NAME,
            "node_type": profile.node_type().as_str(),
            "size": profile.min_nodes(),
            "autoscaling": autoscaling,
            "autohealing": true,
            "zone": layout.zone,
            "tags": tags,
        });
        if autoscaling {
            pool_body["min_size"] = json!(profile.min_nodes());
            pool_body["max_size"] = json!(profile.max_nodes());
        }
        let pool: Pool = session
            .post(&layout.pools(&cluster.id), &pool_body)
            .await
            .map_err(|error| failure("create the node pool", error))?;
        self.record_created(
            session,
            layout,
            id,
            ProvisionedResource {
                kind: ResourceKind::NodePool,
                provider_id: pool.id.clone(),
            },
        )
        .await?;
        info!(data_plane_id = %id, "node pool created");

        self.wait_ready(session, layout, &cluster.id, &pool.id)
            .await?;
        info!(data_plane_id = %id, "cluster ready");

        let kubeconfig = session
            .get_text(
                &format!("{}/kubeconfig", layout.cluster(&cluster.id)),
                &[("dl", "1")],
            )
            .await
            .map_err(|error| failure("download the kubeconfig", error))?;

        let binding = self
            .bootstrapper
            .bootstrap(BootstrapRequest {
                data_plane_id: *id,
                organisation_id,
                region: region.clone(),
                kubeconfig: SecretString::new(kubeconfig),
            })
            .await?;
        info!(data_plane_id = %id, "cluster bootstrapped");
        Ok(binding)
    }

    async fn kubernetes_version(
        &self,
        session: &Session<'_>,
        layout: &Layout,
    ) -> Result<String, CoreError> {
        if let Some(version) = &self.config.kubernetes_version {
            return Ok(version.clone());
        }
        let list: VersionList = session
            .get(&layout.versions(), &[])
            .await
            .map_err(|error| failure("list the kubernetes versions", error))?;
        list.versions
            .into_iter()
            .max_by_key(|version| numeric_key(&version.name))
            .map(|version| version.name)
            .ok_or_else(|| {
                CoreError::InternalError("scaleway offers no kubernetes version".to_string())
            })
    }

    async fn wait_ready(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        cluster_id: &str,
        pool_id: &str,
    ) -> Result<(), CoreError> {
        for attempt in 0..self.config.poll_attempts {
            if attempt > 0 {
                sleep(self.config.poll_interval).await;
            }
            let cluster = session
                .get::<Cluster>(&layout.cluster(cluster_id), &[])
                .await;
            let pool = session.get::<Pool>(&layout.pool(pool_id), &[]).await;
            match (cluster, pool) {
                (Ok(cluster), Ok(pool))
                    if cluster.status == STATUS_READY && pool.status == STATUS_READY =>
                {
                    return Ok(());
                }
                (Ok(_), Ok(_)) => {}
                (Err(error), _) | (_, Err(error)) if error.is_transient() => {}
                (Err(error), _) | (_, Err(error)) => {
                    return Err(failure("read the cluster state", error));
                }
            }
        }
        Err(ProvisionError::NodePoolNeverConverged.into())
    }

    async fn teardown(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        data_plane_id: &DataPlaneId,
        mut resources: Vec<ProvisionedResource>,
    ) -> Result<(), CoreError> {
        resources.sort_by_key(|resource| match resource.kind {
            ResourceKind::NodePool => 0,
            ResourceKind::Cluster => 1,
            ResourceKind::PrivateNetwork => 2,
        });
        for resource in &resources {
            self.delete_resource(session, layout, resource).await?;
            self.inventory
                .mark_released(data_plane_id, resource)
                .await?;
            info!(
                data_plane_id = %data_plane_id,
                kind = resource.kind.as_str(),
                "resource released"
            );
        }
        Ok(())
    }

    async fn delete_resource(
        &self,
        session: &Session<'_>,
        layout: &Layout,
        resource: &ProvisionedResource,
    ) -> Result<(), CoreError> {
        let id = resource.provider_id.as_str();
        let (path, query): (String, &[(&str, &str)]) = match resource.kind {
            ResourceKind::NodePool => (layout.pool(id), &[]),
            ResourceKind::Cluster => (layout.cluster(id), &[("with_additional_resources", "true")]),
            ResourceKind::PrivateNetwork => (layout.private_network(id), &[]),
        };
        match session.delete(&path, query).await {
            Ok(()) => {}
            Err(error) if error.is_not_found() => return Ok(()),
            Err(error) => return Err(failure("delete a resource", error)),
        }
        match resource.kind {
            ResourceKind::NodePool | ResourceKind::Cluster => self.wait_gone(session, &path).await,
            ResourceKind::PrivateNetwork => Ok(()),
        }
    }

    async fn wait_gone(&self, session: &Session<'_>, path: &str) -> Result<(), CoreError> {
        for attempt in 0..self.config.poll_attempts {
            if attempt > 0 {
                sleep(self.config.poll_interval).await;
            }
            match session.get::<Cluster>(path, &[]).await {
                Ok(current) if current.status == STATUS_DELETED => return Ok(()),
                Ok(_) => {}
                Err(error) if error.is_not_found() => return Ok(()),
                Err(error) if error.is_transient() => {}
                Err(error) => return Err(failure("wait for a deletion", error)),
            }
        }
        Err(CoreError::InternalError(format!(
            "scaleway did not finish deleting {path} in time"
        )))
    }

    async fn rollback(&self, session: &Session<'_>, layout: &Layout, id: &DataPlaneId) {
        let outcome = match self.inventory.unreleased(id).await {
            Ok(resources) => self.teardown(session, layout, id, resources).await,
            Err(error) => Err(error),
        };
        if let Err(error) = outcome {
            warn!(data_plane_id = %id, %error, "rollback left resources behind");
        }
    }
}

impl<S, I, R, B> ClusterProvisioner for ScalewayProvisioner<S, I, R, B>
where
    S: CloudCredentialStore,
    I: ClusterInventory,
    R: DataPlaneRepository,
    B: ClusterBootstrapper,
{
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisionedCluster, CoreError> {
        let ProvisionRequest {
            data_plane_id,
            organisation_id,
            region,
            minimum,
            target,
        } = request;
        let ProvisionTarget::Customer {
            credential_id,
            profile,
            ..
        } = target
        else {
            return Err(CoreError::InternalError(
                "the scaleway provisioner only creates clusters in a customer account".to_string(),
            ));
        };

        let session = self.open(&credential_id).await?;
        let layout = Layout::new(&self.config, region.as_str());
        let capacity = self
            .capacity(&session, &layout, &profile, minimum.storage_gib)
            .await?;

        match self
            .build(
                &session,
                &layout,
                &data_plane_id,
                &region,
                organisation_id,
                &profile,
            )
            .await
        {
            Ok(herald) => Ok(ProvisionedCluster { herald, capacity }),
            Err(error) => {
                warn!(data_plane_id = %data_plane_id, %error, "provisioning failed, rolling back");
                self.rollback(&session, &layout, &data_plane_id).await;
                Err(error)
            }
        }
    }

    async fn deprovision(&self, id: &DataPlaneId) -> Result<(), CoreError> {
        let resources = self.inventory.unreleased(id).await?;
        if resources.is_empty() {
            return Ok(());
        }
        let (credential_id, region) = self.locate(id).await?;
        let session = self.open(&credential_id).await?;
        let layout = Layout::new(&self.config, region.as_str());
        self.teardown(&session, &layout, id, resources).await
    }

    async fn resize(&self, id: &DataPlaneId, profile: &ClusterProfile) -> Result<(), CoreError> {
        let resources = self.inventory.unreleased(id).await?;
        let find = |kind: ResourceKind| {
            resources
                .iter()
                .find(|resource| resource.kind == kind)
                .map(|resource| resource.provider_id.as_str())
        };
        let (Some(cluster_id), Some(pool_id)) =
            (find(ResourceKind::Cluster), find(ResourceKind::NodePool))
        else {
            return Err(CoreError::InternalError(format!(
                "data plane {id} has no recorded cluster to resize"
            )));
        };

        let (credential_id, region) = self.locate(id).await?;
        let session = self.open(&credential_id).await?;
        let layout = Layout::new(&self.config, region.as_str());

        let pool: Pool = session
            .get(&layout.pool(pool_id), &[])
            .await
            .map_err(|error| failure("read the node pool", error))?;
        if pool.node_type != profile.node_type().as_str() {
            return Err(CoreError::InternalError(format!(
                "scaleway cannot change the node type of an existing pool from {} to {}",
                pool.node_type,
                profile.node_type().as_str()
            )));
        }

        let cluster: Cluster = session
            .get(&layout.cluster(cluster_id), &[])
            .await
            .map_err(|error| failure("read the cluster", error))?;
        let target_type = profile.control_plane().id.as_str();
        let change_type = cluster.kind != target_type;
        if change_type {
            let available: ClusterTypeList = session
                .get(
                    &format!("{}/available-types", layout.cluster(cluster_id)),
                    &[],
                )
                .await
                .map_err(|error| failure("list the reachable control planes", error))?;
            if !available
                .cluster_types
                .iter()
                .any(|available| available.name == target_type)
            {
                return Err(CoreError::InternalError(format!(
                    "scaleway cannot move this cluster from control plane {} to {target_type}",
                    cluster.kind
                )));
            }
        }

        let autoscaling = profile.mode().limits().autoscaling;
        let min = u32::from(profile.min_nodes());
        let max = u32::from(profile.max_nodes());
        let mut body = json!({
            "autoscaling": autoscaling,
            "size": if autoscaling { pool.size.max(min).min(max) } else { min },
        });
        if autoscaling {
            body["min_size"] = json!(min);
            body["max_size"] = json!(max);
        }
        let _: Pool = session
            .patch(&layout.pool(pool_id), &body)
            .await
            .map_err(|error| failure("resize the node pool", error))?;

        if change_type {
            let _: Cluster = session
                .post(
                    &format!("{}/set-type", layout.cluster(cluster_id)),
                    &json!({ "type": target_type }),
                )
                .await
                .map_err(|error| failure("change the control plane", error))?;
        }
        Ok(())
    }
}

fn failure(step: &str, error: ApiError) -> CoreError {
    if error.has_kind("quotas_exceeded") {
        ProvisionError::QuotaExceeded.into()
    } else if error.has_kind("out_of_stock") || error.mentions_location() {
        ProvisionError::RegionUnavailable.into()
    } else if error.is_denied() {
        ProvisionError::CredentialRejected.into()
    } else {
        CoreError::InternalError(format!("scaleway could not {step}: {error}"))
    }
}

fn numeric_key(name: &str) -> Vec<u64> {
    name.split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}
