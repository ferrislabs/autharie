use serde_json::{Value, json};

use crate::{
    CoreError,
    action::{
        Action, ActionPayload, ActionSource, ActionTarget, ActionType, ActionVersion, TargetKind,
        commands::RecordActionCommand,
        ports::{ActionRepository, ActionService},
        service::ActionServiceImpl,
    },
    deployments::{
        Deployment, DeploymentId, DeploymentStatus, commands::DeploymentOutcome,
        distribution::Distribution, ports::DeploymentRepository,
    },
};

use super::{Cell, CellError, CellId, CellRepository, RealmName};

pub const REALM_CREATE: &str = "realm.create";
pub const REALM_DELETE: &str = "realm.delete";

const UNSTATED_REFUSAL: &str = "the cell refused the realm";

pub struct RealmRequests<C, D, A>
where
    C: CellRepository,
    D: DeploymentRepository,
    A: ActionRepository,
{
    cells: C,
    deployments: D,
    actions: ActionServiceImpl<A>,
}

impl<C, D, A> RealmRequests<C, D, A>
where
    C: CellRepository,
    D: DeploymentRepository,
    A: ActionRepository,
{
    pub fn new(cells: C, deployments: D, actions: A) -> Self {
        Self {
            cells,
            deployments,
            actions: ActionServiceImpl::new(actions),
        }
    }

    pub async fn create(
        &self,
        deployment: &Deployment,
        hostname: &str,
    ) -> Result<Action, CoreError> {
        let (cell_id, realm) = pooled(deployment)?;
        if !names_realm(hostname, realm) {
            return Err(CoreError::InternalError(format!(
                "hostname '{hostname}' does not name realm '{realm}'"
            )));
        }

        let payload = self.payload(deployment, cell_id, realm).await?;
        self.record(
            deployment,
            REALM_CREATE,
            with_hostname(payload, hostname),
            ActionSource::User {
                user_id: deployment.created_by.0,
            },
        )
        .await
    }

    pub async fn delete(&self, deployment: &Deployment) -> Result<Action, CoreError> {
        let (cell_id, realm) = pooled(deployment)?;
        let payload = self.payload(deployment, cell_id, realm).await?;
        self.record(deployment, REALM_DELETE, payload, ActionSource::System)
            .await
    }

    async fn payload(
        &self,
        deployment: &Deployment,
        cell_id: CellId,
        realm: &RealmName,
    ) -> Result<Value, CoreError> {
        let cell = self
            .cells
            .find(cell_id)
            .await?
            .ok_or(CellError::UnknownCell { id: cell_id.0 })?;
        if cell.data_plane_id() != deployment.dataplane_id {
            return Err(CellError::Inconsistent {
                reason: format!(
                    "deployment {} is not on the data plane of cell {cell_id}",
                    deployment.id.0
                ),
            }
            .into());
        }

        let instance = self.instance_of(&cell).await?;
        Ok(json!({
            "deployment_id": deployment.id.0,
            "cell": {
                "deployment_id": instance.id.0,
                "namespace": instance.namespace,
            },
            "realm": realm.as_str(),
        }))
    }

    async fn instance_of(&self, cell: &Cell) -> Result<Deployment, CoreError> {
        self.deployments
            .get_by_id(cell.instance())
            .await?
            .ok_or_else(|| {
                CellError::Inconsistent {
                    reason: format!("cell {} has no instance deployment", cell.id()),
                }
                .into()
            })
    }

    async fn record(
        &self,
        deployment: &Deployment,
        action_type: &str,
        payload: Value,
        source: ActionSource,
    ) -> Result<Action, CoreError> {
        self.actions
            .record_action(RecordActionCommand::new(
                deployment.id,
                deployment.dataplane_id,
                ActionType(action_type.to_string()),
                ActionTarget {
                    kind: TargetKind::Realm,
                    id: deployment.id.0,
                },
                ActionPayload { data: payload },
                ActionVersion(1),
                source,
            ))
            .await
    }
}

fn pooled(deployment: &Deployment) -> Result<(CellId, &RealmName), CoreError> {
    match &deployment.distribution {
        Distribution::Pooled { cell_id, realm } => Ok((*cell_id, realm)),
        Distribution::Shared | Distribution::SelfHosted | Distribution::CustomerCloud { .. } => {
            Err(CoreError::InternalError(format!(
                "deployment {} is not a realm in a cell",
                deployment.id.0
            )))
        }
    }
}

fn names_realm(hostname: &str, realm: &RealmName) -> bool {
    hostname
        .split_once('.')
        .is_some_and(|(label, zone)| label == realm.as_str() && !zone.is_empty())
}

fn with_hostname(mut payload: Value, hostname: &str) -> Value {
    payload["hostname"] = json!(hostname);
    payload
}

pub struct PooledOutcomes<C, D>
where
    C: CellRepository,
    D: DeploymentRepository,
{
    cells: C,
    deployments: D,
}

impl<C, D> PooledOutcomes<C, D>
where
    C: CellRepository,
    D: DeploymentRepository,
{
    pub fn new(cells: C, deployments: D) -> Self {
        Self { cells, deployments }
    }

    pub async fn settle(
        &self,
        deployment_id: DeploymentId,
        outcome: DeploymentOutcome,
        reason: Option<&str>,
    ) -> Result<bool, CoreError> {
        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(false);
        };
        if !matches!(deployment.distribution, Distribution::Pooled { .. }) {
            return Ok(false);
        }

        match (outcome, deployment.status) {
            (DeploymentOutcome::Failed, DeploymentStatus::Failed) => {
                self.cells
                    .release_refused(deployment.id, reason.unwrap_or(UNSTATED_REFUSAL))
                    .await
            }
            (DeploymentOutcome::Deleted, DeploymentStatus::Deleted) => {
                self.cells.release(deployment.id).await
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use uuid::Uuid;

    use super::*;
    use crate::{
        action::ports::MockActionRepository,
        cells::{CellPolicy, CellStatus, ports::MockCellRepository},
        dataplane::value_objects::{DataPlaneId, DeploymentResources, Region},
        deployments::{
            DeploymentKind, DeploymentName, environment::Environment, network::NetworkAccess,
            ports::MockDeploymentRepository,
        },
        organisation::OrganisationId,
        user::UserId,
        version::Version,
    };

    const PLANE: DataPlaneId = DataPlaneId(Uuid::from_u128(7));
    const CELL: CellId = CellId(Uuid::from_u128(8));
    const INSTANCE: DeploymentId = DeploymentId(Uuid::from_u128(9));
    const TENANT: DeploymentId = DeploymentId(Uuid::from_u128(1));

    fn deployment(id: DeploymentId, name: &str, distribution: Distribution) -> Deployment {
        let at = Utc::now();
        Deployment {
            topology: Default::default(),
            id,
            organisation_id: OrganisationId(Uuid::from_u128(3)),
            dataplane_id: PLANE,
            name: DeploymentName(name.to_string()),
            kind: DeploymentKind::Ferriskey,
            version: Version::new(26, 0, 1),
            status: DeploymentStatus::Pending,
            namespace: format!("cells-{name}"),
            environment: Environment::Development,
            offer: None,
            restored_from: None,
            resources: DeploymentResources::DEFAULT,
            created_by: UserId(Uuid::from_u128(4)),
            created_at: at,
            updated_at: at,
            deployed_at: None,
            deleted_at: None,
            auto_upgrade: Default::default(),
            maintenance_window: None,
            network_access: NetworkAccess::Open,
            last_verified_restore_at: None,
            last_restore_drill_seconds: None,
            log_shipping_enabled: false,
            iam_settings: Default::default(),
            distribution,
        }
    }

    fn tenant(status: DeploymentStatus) -> Deployment {
        let mut tenant = deployment(
            TENANT,
            "acme",
            Distribution::Pooled {
                cell_id: CELL,
                realm: RealmName::try_from("acme").expect("valid"),
            },
        );
        tenant.status = status;
        tenant
    }

    fn cell(plane: DataPlaneId) -> Cell {
        let mut cell = Cell::new(
            CELL,
            Region::new("eu"),
            plane,
            INSTANCE,
            &CellPolicy::new(200, 50).expect("valid"),
            Utc::now(),
        );
        cell.open().expect("opens");
        assert_eq!(cell.status(), CellStatus::Open);
        cell
    }

    fn cells_finding(found: Option<Cell>) -> MockCellRepository {
        let mut cells = MockCellRepository::new();
        cells.expect_find().returning(move |_| {
            let found = found.clone();
            Box::pin(async move { Ok(found) })
        });
        cells
    }

    fn deployments_finding(found: Option<Deployment>) -> MockDeploymentRepository {
        let mut deployments = MockDeploymentRepository::new();
        deployments.expect_get_by_id().returning(move |_| {
            let found = found.clone();
            Box::pin(async move { Ok(found) })
        });
        deployments
    }

    type Recorded = Arc<Mutex<Vec<Action>>>;
    type Requests =
        RealmRequests<MockCellRepository, MockDeploymentRepository, MockActionRepository>;

    fn requests(cell_found: Option<Cell>) -> (Requests, Recorded) {
        let recorded: Recorded = Arc::default();
        let sink = Arc::clone(&recorded);
        let mut actions = MockActionRepository::new();
        actions.expect_append().returning(move |action| {
            sink.lock().expect("not poisoned").push(action);
            Box::pin(async { Ok(()) })
        });
        let instance = deployment(INSTANCE, "cell-one", Distribution::Shared);

        (
            RealmRequests::new(
                cells_finding(cell_found),
                deployments_finding(Some(instance)),
                actions,
            ),
            recorded,
        )
    }

    fn recorded_count(recorded: &Recorded) -> usize {
        recorded.lock().expect("not poisoned").len()
    }

    #[tokio::test]
    async fn a_realm_create_names_the_cell_instance_the_realm_and_the_hostname() {
        let (requests, recorded) = requests(Some(cell(PLANE)));

        let action = requests
            .create(&tenant(DeploymentStatus::Pending), "acme.cells.example")
            .await
            .expect("recorded");

        assert_eq!(action.action_type, ActionType(REALM_CREATE.to_string()));
        assert_eq!(action.deployment_id, Some(TENANT));
        assert_eq!(action.dataplane_id, PLANE);
        assert_eq!(action.target.kind, TargetKind::Realm);
        assert_eq!(
            action.payload.data,
            json!({
                "deployment_id": TENANT.0,
                "cell": { "deployment_id": INSTANCE.0, "namespace": "cells-cell-one" },
                "realm": "acme",
                "hostname": "acme.cells.example",
            })
        );
        assert_eq!(recorded_count(&recorded), 1);
    }

    #[tokio::test]
    async fn a_realm_delete_carries_no_hostname() {
        let (requests, _) = requests(Some(cell(PLANE)));

        let action = requests
            .delete(&tenant(DeploymentStatus::Deleting))
            .await
            .expect("recorded");

        assert_eq!(action.action_type, ActionType(REALM_DELETE.to_string()));
        assert_eq!(action.metadata.source, ActionSource::System);
        assert!(action.payload.data.get("hostname").is_none());
        assert_eq!(action.payload.data["realm"], json!("acme"));
    }

    #[tokio::test]
    async fn a_deployment_that_is_not_pooled_has_no_realm_to_create() {
        let (requests, recorded) = requests(Some(cell(PLANE)));
        let shared = deployment(TENANT, "acme", Distribution::Shared);

        let error = requests
            .create(&shared, "acme.cells.example")
            .await
            .expect_err("refused");

        assert!(matches!(error, CoreError::InternalError(_)));
        assert_eq!(recorded_count(&recorded), 0);
    }

    #[tokio::test]
    async fn a_hostname_that_does_not_name_the_realm_is_refused() {
        for hostname in [
            "other.cells.example",
            "acme",
            "acme.",
            "xacme.cells.example",
        ] {
            let (requests, recorded) = requests(Some(cell(PLANE)));

            let result = requests
                .create(&tenant(DeploymentStatus::Pending), hostname)
                .await;

            assert!(result.is_err(), "{hostname}");
            assert_eq!(recorded_count(&recorded), 0);
        }
    }

    #[tokio::test]
    async fn an_unknown_cell_records_nothing() {
        let (requests, recorded) = requests(None);

        let error = requests
            .create(&tenant(DeploymentStatus::Pending), "acme.cells.example")
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            CoreError::Cell(CellError::UnknownCell { id }) if id == CELL.0
        ));
        assert_eq!(recorded_count(&recorded), 0);
    }

    #[tokio::test]
    async fn a_tenant_on_another_data_plane_than_its_cell_is_inconsistent() {
        let (requests, recorded) = requests(Some(cell(DataPlaneId(Uuid::from_u128(99)))));

        let error = requests
            .create(&tenant(DeploymentStatus::Pending), "acme.cells.example")
            .await
            .expect_err("refused");

        assert!(matches!(
            error,
            CoreError::Cell(CellError::Inconsistent { .. })
        ));
        assert_eq!(recorded_count(&recorded), 0);
    }

    fn outcomes(
        found: Deployment,
        cells: MockCellRepository,
    ) -> PooledOutcomes<MockCellRepository, MockDeploymentRepository> {
        PooledOutcomes::new(cells, deployments_finding(Some(found)))
    }

    #[tokio::test]
    async fn a_refusal_releases_the_slot_with_its_reason() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_release_refused()
            .withf(|deployment, reason| *deployment == TENANT && reason == "name refused")
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(true) }));

        let released = outcomes(tenant(DeploymentStatus::Failed), cells)
            .settle(TENANT, DeploymentOutcome::Failed, Some("name refused"))
            .await
            .expect("settled");

        assert!(released);
    }

    #[tokio::test]
    async fn a_refusal_without_a_reason_still_says_why() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_release_refused()
            .withf(|_, reason| reason == UNSTATED_REFUSAL)
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(true) }));

        outcomes(tenant(DeploymentStatus::Failed), cells)
            .settle(TENANT, DeploymentOutcome::Failed, None)
            .await
            .expect("settled");
    }

    #[tokio::test]
    async fn a_deleted_realm_releases_the_slot() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_release()
            .withf(|deployment| *deployment == TENANT)
            .times(1)
            .returning(|_| Box::pin(async { Ok(true) }));

        let released = outcomes(tenant(DeploymentStatus::Deleted), cells)
            .settle(TENANT, DeploymentOutcome::Deleted, None)
            .await
            .expect("settled");

        assert!(released);
    }

    #[tokio::test]
    async fn nothing_is_released_unless_the_status_moved_to_match() {
        let cases = [
            (DeploymentStatus::Successful, DeploymentOutcome::Running),
            (DeploymentStatus::Successful, DeploymentOutcome::Failed),
            (DeploymentStatus::Deleting, DeploymentOutcome::Deleted),
            (DeploymentStatus::Deleting, DeploymentOutcome::Failed),
        ];

        for (status, outcome) in cases {
            let released = outcomes(tenant(status.clone()), MockCellRepository::new())
                .settle(TENANT, outcome, Some("x"))
                .await
                .expect("settled");

            assert!(!released, "{status:?} {outcome:?}");
        }
    }

    #[tokio::test]
    async fn a_deployment_that_is_not_pooled_holds_no_slot() {
        let mut shared = deployment(TENANT, "acme", Distribution::Shared);
        shared.status = DeploymentStatus::Deleted;

        let released = outcomes(shared, MockCellRepository::new())
            .settle(TENANT, DeploymentOutcome::Deleted, None)
            .await
            .expect("settled");

        assert!(!released);
    }
}
