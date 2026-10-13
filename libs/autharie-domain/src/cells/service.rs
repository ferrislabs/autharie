use autharie_auth::Identity;
use chrono::Utc;
use uuid::Uuid;

use crate::{
    CoreError,
    catalog::{Release, ReleaseStatus},
    dataplane::value_objects::{DeploymentResources, Region},
    deployments::{
        Deployment, DeploymentKind, DeploymentName, DeploymentStatus,
        commands::CreateDeploymentCommand, environment::Environment, topology::Topology,
    },
    offers::Offer,
    organisation::OrganisationId,
    platform::{PlatformRight, ports::PlatformPolicy},
    user::UserId,
    version::Version,
};

use super::{Cell, CellError, CellId, CellPolicy, CellRepository, CellStatus, HighAvailability};

pub const CELL_DATABASE_INSTANCE: DeploymentResources = DeploymentResources {
    cpu_millis: 1000,
    memory_mib: 2048,
    storage_gib: 10,
};

pub const CELL_API_REPLICA: DeploymentResources = DeploymentResources {
    cpu_millis: 250,
    memory_mib: 256,
    storage_gib: 0,
};

pub const CELL_WEBAPP_REPLICA: DeploymentResources = DeploymentResources {
    cpu_millis: 50,
    memory_mib: 64,
    storage_gib: 0,
};

pub fn cell_reservation(topology: Topology) -> DeploymentResources {
    let replicas = u32::from(topology.replicas());
    let databases = u32::from(topology.database_instances());
    let total =
        |database: u32, api: u32, webapp: u32| database * databases + (api + webapp) * replicas;

    DeploymentResources {
        cpu_millis: total(
            CELL_DATABASE_INSTANCE.cpu_millis,
            CELL_API_REPLICA.cpu_millis,
            CELL_WEBAPP_REPLICA.cpu_millis,
        ),
        memory_mib: total(
            CELL_DATABASE_INSTANCE.memory_mib,
            CELL_API_REPLICA.memory_mib,
            CELL_WEBAPP_REPLICA.memory_mib,
        ),
        storage_gib: total(
            CELL_DATABASE_INSTANCE.storage_gib,
            CELL_API_REPLICA.storage_gib,
            CELL_WEBAPP_REPLICA.storage_gib,
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCellCommand {
    id: CellId,
    region: Region,
    policy: CellPolicy,
}

impl CreateCellCommand {
    pub fn new(region: Region, capacity: Option<u16>) -> Result<Self, CellError> {
        let default = CellPolicy::default();
        let policy = match capacity {
            None => default,
            Some(capacity) if capacity > default.capacity() => {
                return Err(CellError::CapacityInvalid);
            }
            Some(capacity) => CellPolicy::new(
                capacity,
                default.spare_slots().min(capacity.saturating_sub(1)),
            )?,
        };

        Ok(Self {
            id: CellId(Uuid::new_v4()),
            region,
            policy,
        })
    }

    pub fn id(&self) -> CellId {
        self.id
    }

    pub fn region(&self) -> &Region {
        &self.region
    }

    pub fn policy(&self) -> &CellPolicy {
        &self.policy
    }

    pub fn instance(
        &self,
        organisation_id: OrganisationId,
        version: Version,
        created_by: UserId,
    ) -> Result<CreateDeploymentCommand, CoreError> {
        let name = format!("cell-{}", &self.id.0.simple().to_string()[..8]);

        Ok(CreateDeploymentCommand::new(
            organisation_id,
            DeploymentName(name),
            DeploymentKind::Ferriskey,
            version,
            created_by,
            Environment::Production,
            self.region.clone(),
            Offer::Standard,
        )?
        .with_topology(Topology::HIGH_AVAILABILITY)
        .sized_by_platform(cell_reservation(Topology::HIGH_AVAILABILITY)))
    }
}

pub fn release_for_cells(releases: &[Release]) -> Result<Version, CellError> {
    releases
        .iter()
        .filter(|release| {
            release.id.kind == DeploymentKind::Ferriskey
                && release.status == ReleaseStatus::Available
        })
        .map(|release| &release.id.version)
        .max()
        .cloned()
        .ok_or(CellError::NoRelease)
}

pub struct CellServiceImpl<C, P>
where
    C: CellRepository,
    P: PlatformPolicy,
{
    cells: C,
    policy: P,
}

impl<C, P> CellServiceImpl<C, P>
where
    C: CellRepository,
    P: PlatformPolicy,
{
    pub fn new(cells: C, policy: P) -> Self {
        Self { cells, policy }
    }

    pub async fn authorise_creation(&self, identity: Identity) -> Result<(), CoreError> {
        self.policy
            .require(identity, PlatformRight::OperateFleet)
            .await
    }

    pub async fn record(
        &self,
        command: &CreateCellCommand,
        instance: &Deployment,
    ) -> Result<Cell, CoreError> {
        let cell = Cell::new(
            command.id(),
            command.region().clone(),
            instance.dataplane_id,
            instance.id,
            command.policy(),
            Utc::now(),
        );
        self.cells.create(&cell).await?;

        Ok(cell)
    }

    pub async fn list(&self, identity: Identity) -> Result<Vec<Cell>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;

        self.cells.list().await
    }

    pub async fn get(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;

        self.find(id).await
    }

    pub async fn drain(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        self.policy
            .require(identity, PlatformRight::OperateFleet)
            .await?;

        let mut cell = self.find(id).await?;
        cell.drain()?;
        self.cells.set_status(id, cell.status()).await?;

        Ok(cell)
    }

    pub async fn retire(&self, identity: Identity, id: CellId) -> Result<Cell, CoreError> {
        self.policy
            .require(identity, PlatformRight::OperateFleet)
            .await?;

        let mut cell = self.find(id).await?;
        cell.retire()?;
        self.cells.set_status(id, cell.status()).await?;

        Ok(cell)
    }

    pub async fn settle_instance(&self, instance: &Deployment) -> Result<Option<Cell>, CoreError> {
        let Some(mut cell) = self.cells.find_by_instance(instance.id).await? else {
            return Ok(None);
        };
        if cell.status() != CellStatus::Provisioning {
            return Ok(None);
        }

        match (&instance.status, HighAvailability::proven_by(instance)) {
            (_, Some(proof)) => cell.open(proof)?,
            (
                DeploymentStatus::Failed | DeploymentStatus::Deleting | DeploymentStatus::Deleted,
                None,
            ) => cell.fail(),
            _ => return Ok(None),
        }
        self.cells.set_status(cell.id(), cell.status()).await?;

        Ok(Some(cell))
    }

    async fn find(&self, id: CellId) -> Result<Cell, CoreError> {
        self.cells
            .find(id)
            .await?
            .ok_or_else(|| CellError::UnknownCell { id: id.0 }.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::DateTime;

    use super::*;
    use crate::{
        catalog::{BreakingRisk, ReleaseId, ReleaseNotes, Rollout},
        cells::{MockCellRepository, fixtures::instance},
        dataplane::value_objects::DataPlaneId,
        deployments::{DeploymentId, commands::Sizing},
        platform::fixtures::Granting,
    };

    fn identity() -> Identity {
        Identity::Client(autharie_auth::Client {
            id: "id".to_string(),
            client_id: "operator".to_string(),
            roles: vec![],
            scopes: vec![],
        })
    }

    fn any_cell() -> CellId {
        CellId(Uuid::new_v4())
    }

    fn stored(status: CellStatus, instance: DeploymentId, realms: u16) -> Cell {
        Cell::restore(
            CellId(Uuid::new_v4()),
            Region::new("fr-par"),
            DataPlaneId(Uuid::new_v4()),
            instance,
            status,
            200,
            realms,
            DateTime::<Utc>::UNIX_EPOCH,
        )
        .expect("a consistent cell")
    }

    type Written = Arc<Mutex<Vec<CellStatus>>>;

    fn holding(cell: Cell) -> (MockCellRepository, Written) {
        let written = Written::default();
        let mut cells = MockCellRepository::new();
        let found = cell.clone();
        cells
            .expect_find()
            .returning(move |_| Box::pin(std::future::ready(Ok(Some(found.clone())))));
        cells
            .expect_find_by_instance()
            .returning(move |_| Box::pin(std::future::ready(Ok(Some(cell.clone())))));
        let sink = Arc::clone(&written);
        cells.expect_set_status().returning(move |_, status| {
            sink.lock().expect("lock").push(status);
            Box::pin(std::future::ready(Ok(())))
        });

        (cells, written)
    }

    fn written(written: &Written) -> Vec<CellStatus> {
        written.lock().expect("lock").clone()
    }

    fn release(version: Version, status: ReleaseStatus) -> Release {
        Release {
            id: ReleaseId::new(DeploymentKind::Ferriskey, version),
            status,
            risk: BreakingRisk::None,
            notes: ReleaseNotes(String::new()),
            rollout: Rollout::full(),
            minimum_operator_version: None,
            steps_through: Vec::new(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    async fn settle(cell: Cell, reported: &Deployment) -> (Option<Cell>, Vec<CellStatus>) {
        let (cells, sink) = holding(cell);
        let settled = CellServiceImpl::new(cells, Granting::nothing())
            .settle_instance(reported)
            .await
            .expect("settled");

        (settled, written(&sink))
    }

    #[test]
    fn a_cell_holds_two_hundred_realms_unless_asked_for_fewer() {
        let region = Region::new("fr-par");
        let capacity = |asked| {
            CreateCellCommand::new(region.clone(), asked).map(|command| command.policy().capacity())
        };

        assert_eq!(capacity(None), Ok(200));
        assert_eq!(capacity(Some(10)), Ok(10));
        assert_eq!(capacity(Some(1)), Ok(1));
        assert_eq!(capacity(Some(0)), Err(CellError::CapacityInvalid));
        assert_eq!(capacity(Some(201)), Err(CellError::CapacityInvalid));
    }

    /// @spec-fpr-11
    #[test]
    fn the_instance_of_a_cell_is_a_highly_available_ferriskey_sized_by_the_platform() {
        let command = CreateCellCommand::new(Region::new("fr-par"), None).expect("a command");
        let instance = command
            .instance(
                OrganisationId(Uuid::new_v4()),
                Version::new(1, 0, 0),
                UserId(Uuid::new_v4()),
            )
            .expect("an instance command");

        assert_eq!(instance.kind, DeploymentKind::Ferriskey);
        assert_eq!(instance.topology(), Topology::HIGH_AVAILABILITY);
        assert_eq!(
            instance.sizing(),
            Sizing::Platform(cell_reservation(Topology::HIGH_AVAILABILITY))
        );
        assert_eq!(instance.region.as_str(), "fr-par");
        assert!(instance.name.0.starts_with("cell-"), "{}", instance.name.0);
    }

    #[test]
    fn a_cell_is_built_on_the_newest_available_ferriskey_release() {
        let releases = [
            release(Version::new(3, 0, 0), ReleaseStatus::Upcoming),
            release(Version::new(2, 1, 0), ReleaseStatus::Available),
            release(Version::new(2, 2, 0), ReleaseStatus::Withdrawn),
            release(Version::new(1, 9, 0), ReleaseStatus::Available),
            release(Version::new(2, 0, 0), ReleaseStatus::Deprecated),
        ];

        assert_eq!(release_for_cells(&releases), Ok(Version::new(2, 1, 0)));
        assert_eq!(release_for_cells(&releases[..1]), Err(CellError::NoRelease));
    }

    /// @spec-fpr-12
    #[tokio::test]
    async fn a_new_cell_is_provisioning_on_the_plane_of_its_instance() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_create()
            .withf(|cell| cell.status() == CellStatus::Provisioning)
            .times(1)
            .returning(|_| Box::pin(std::future::ready(Ok(()))));
        let command = CreateCellCommand::new(Region::new("fr-par"), Some(50)).expect("a command");
        let deployment = instance(
            DeploymentId(Uuid::new_v4()),
            DeploymentStatus::Pending,
            Topology::HIGH_AVAILABILITY,
        );

        let cell = CellServiceImpl::new(cells, Granting::everything())
            .record(&command, &deployment)
            .await
            .expect("recorded");

        assert_eq!(cell.id(), command.id());
        assert_eq!(cell.instance(), deployment.id);
        assert_eq!(cell.data_plane_id(), deployment.dataplane_id);
        assert_eq!(cell.capacity(), 50);
        assert_eq!(cell.status(), CellStatus::Provisioning);
    }

    /// @spec-fpr-12
    #[tokio::test]
    async fn a_provisioning_cell_opens_when_its_instance_serves_highly_available() {
        let id = DeploymentId(Uuid::new_v4());

        let (settled, written) = settle(
            stored(CellStatus::Provisioning, id, 0),
            &instance(
                id,
                DeploymentStatus::Successful,
                Topology::HIGH_AVAILABILITY,
            ),
        )
        .await;

        assert_eq!(settled.map(|cell| cell.status()), Some(CellStatus::Open));
        assert_eq!(written, vec![CellStatus::Open]);
    }

    /// @spec-fpr-11
    #[tokio::test]
    async fn a_cell_whose_instance_serves_without_high_availability_stays_provisioning() {
        let id = DeploymentId(Uuid::new_v4());

        for topology in [Topology::SINGLE, Topology::new(2, 1).expect("a topology")] {
            let (settled, written) = settle(
                stored(CellStatus::Provisioning, id, 0),
                &instance(id, DeploymentStatus::Successful, topology),
            )
            .await;

            assert_eq!(settled, None, "{topology:?}");
            assert!(written.is_empty(), "{topology:?}");
        }
    }

    #[tokio::test]
    async fn a_provisioning_cell_whose_instance_failed_is_failed() {
        let id = DeploymentId(Uuid::new_v4());

        let (settled, written) = settle(
            stored(CellStatus::Provisioning, id, 0),
            &instance(id, DeploymentStatus::Failed, Topology::HIGH_AVAILABILITY),
        )
        .await;

        assert_eq!(settled.map(|cell| cell.status()), Some(CellStatus::Failed));
        assert_eq!(written, vec![CellStatus::Failed]);
    }

    #[test]
    fn a_cell_reserves_every_database_instance_and_every_replica_of_its_instance() {
        assert_eq!(
            cell_reservation(Topology::HIGH_AVAILABILITY),
            DeploymentResources {
                cpu_millis: 2 * 1000 + 2 * (250 + 50),
                memory_mib: 2 * 2048 + 2 * (256 + 64),
                storage_gib: 2 * 10,
            }
        );
        assert_eq!(
            cell_reservation(Topology::new(3, 2).expect("a topology")).cpu_millis,
            2 * 1000 + 3 * (250 + 50)
        );
    }

    #[tokio::test]
    async fn a_provisioning_cell_whose_instance_is_torn_down_is_failed() {
        let id = DeploymentId(Uuid::new_v4());

        for status in [DeploymentStatus::Deleting, DeploymentStatus::Deleted] {
            let (settled, written) = settle(
                stored(CellStatus::Provisioning, id, 0),
                &instance(id, status.clone(), Topology::HIGH_AVAILABILITY),
            )
            .await;

            assert_eq!(
                settled.map(|cell| cell.status()),
                Some(CellStatus::Failed),
                "{status:?}"
            );
            assert_eq!(written, vec![CellStatus::Failed]);
        }
    }

    #[tokio::test]
    async fn a_failed_cell_whose_instance_comes_up_later_stays_failed() {
        let id = DeploymentId(Uuid::new_v4());

        let (settled, written) = settle(
            stored(CellStatus::Failed, id, 0),
            &instance(
                id,
                DeploymentStatus::Successful,
                Topology::HIGH_AVAILABILITY,
            ),
        )
        .await;

        assert_eq!(settled, None);
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn a_provisioning_cell_waits_while_its_instance_is_on_its_way() {
        let id = DeploymentId(Uuid::new_v4());

        let (settled, written) = settle(
            stored(CellStatus::Provisioning, id, 0),
            &instance(
                id,
                DeploymentStatus::InProgress,
                Topology::HIGH_AVAILABILITY,
            ),
        )
        .await;

        assert_eq!(settled, None);
        assert!(written.is_empty());
    }

    #[tokio::test]
    async fn a_report_about_a_cell_past_provisioning_changes_nothing() {
        let id = DeploymentId(Uuid::new_v4());

        for status in [DeploymentStatus::Failed, DeploymentStatus::Successful] {
            let (settled, written) = settle(
                stored(CellStatus::Open, id, 3),
                &instance(id, status.clone(), Topology::HIGH_AVAILABILITY),
            )
            .await;

            assert_eq!(settled, None, "{status:?}");
            assert!(written.is_empty(), "{status:?}");
        }
    }

    #[tokio::test]
    async fn a_report_about_a_deployment_that_is_no_cell_changes_nothing() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_find_by_instance()
            .returning(|_| Box::pin(std::future::ready(Ok(None))));
        cells.expect_set_status().never();

        let settled = CellServiceImpl::new(cells, Granting::nothing())
            .settle_instance(&instance(
                DeploymentId(Uuid::new_v4()),
                DeploymentStatus::Successful,
                Topology::HIGH_AVAILABILITY,
            ))
            .await
            .expect("settled");

        assert_eq!(settled, None);
    }

    #[tokio::test]
    async fn an_operator_drains_an_open_cell() {
        let (cells, sink) = holding(stored(CellStatus::Open, DeploymentId(Uuid::new_v4()), 3));

        let drained = CellServiceImpl::new(cells, Granting::only(PlatformRight::OperateFleet))
            .drain(identity(), any_cell())
            .await
            .expect("drained");

        assert_eq!(drained.status(), CellStatus::Draining);
        assert_eq!(written(&sink), vec![CellStatus::Draining]);
    }

    #[tokio::test]
    async fn a_provisioning_cell_cannot_be_drained() {
        let (cells, sink) = holding(stored(
            CellStatus::Provisioning,
            DeploymentId(Uuid::new_v4()),
            0,
        ));

        let refused = CellServiceImpl::new(cells, Granting::everything())
            .drain(identity(), any_cell())
            .await
            .expect_err("a provisioning cell was drained");

        assert!(
            matches!(refused, CoreError::Cell(CellError::InvalidTransition)),
            "{refused}"
        );
        assert!(written(&sink).is_empty());
    }

    #[tokio::test]
    async fn a_draining_cell_retires_only_once_it_holds_no_realm() {
        let (empty, sink) = holding(stored(
            CellStatus::Draining,
            DeploymentId(Uuid::new_v4()),
            0,
        ));
        let retired = CellServiceImpl::new(empty, Granting::only(PlatformRight::OperateFleet))
            .retire(identity(), any_cell())
            .await
            .expect("retired");
        assert_eq!(retired.status(), CellStatus::Retired);
        assert_eq!(written(&sink), vec![CellStatus::Retired]);

        let (occupied, sink) = holding(stored(
            CellStatus::Draining,
            DeploymentId(Uuid::new_v4()),
            4,
        ));
        let refused = CellServiceImpl::new(occupied, Granting::everything())
            .retire(identity(), any_cell())
            .await
            .expect_err("a cell holding realms was retired");
        assert!(
            matches!(refused, CoreError::Cell(CellError::InvalidTransition)),
            "{refused}"
        );
        assert!(written(&sink).is_empty());
    }

    #[tokio::test]
    async fn an_unknown_cell_is_named_as_unknown() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_find()
            .returning(|_| Box::pin(std::future::ready(Ok(None))));
        let id = any_cell();

        let refused = CellServiceImpl::new(cells, Granting::everything())
            .get(identity(), id)
            .await
            .expect_err("an unknown cell was found");

        assert!(
            matches!(refused, CoreError::Cell(CellError::UnknownCell { id: unknown }) if unknown == id.0),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn seeing_the_cells_is_not_permission_to_change_them() {
        let (cells, sink) = holding(stored(CellStatus::Open, DeploymentId(Uuid::new_v4()), 0));
        let service = CellServiceImpl::new(cells, Granting::only(PlatformRight::ViewEstate));

        assert!(service.get(identity(), any_cell()).await.is_ok());
        let refusals = [
            service.drain(identity(), any_cell()).await.err(),
            service.retire(identity(), any_cell()).await.err(),
            service.authorise_creation(identity()).await.err(),
        ];

        for refused in refusals {
            assert!(
                matches!(&refused, Some(CoreError::MissingPlatformRight { right }) if right == "operate_fleet"),
                "{refused:?}"
            );
        }
        assert!(written(&sink).is_empty());
    }

    #[tokio::test]
    async fn listing_the_cells_needs_the_right_to_see_the_estate() {
        let mut cells = MockCellRepository::new();
        cells.expect_list().never();

        let refused = CellServiceImpl::new(cells, Granting::nothing())
            .list(identity())
            .await
            .expect_err("somebody with no right listed the cells");

        assert!(
            matches!(&refused, CoreError::MissingPlatformRight { right } if right == "view_estate"),
            "{refused}"
        );
    }
}
