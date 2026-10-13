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
use crate::dns::hostname_for;

pub const REALM_CREATE: &str = "realm.create";
pub const REALM_DELETE: &str = "realm.delete";
pub const REALM_ADMINISTRATOR_ISSUE: &str = "realm.administrator.issue";

const UNSTATED_REFUSAL: &str = "the cell refused the realm";
pub const HAND_OFF_FAILED: &str = "the realm request could not be handed to the data plane";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    Unchanged,
    Released,
    RealmOutlivedTenant(Box<Deployment>),
}

impl Settlement {
    fn released(released: bool) -> Self {
        if released {
            Self::Released
        } else {
            Self::Unchanged
        }
    }
}

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

    pub async fn create(&self, deployment: &Deployment, zone: &str) -> Result<Action, CoreError> {
        let (cell_id, realm) = pooled(deployment)?;
        let hostname = realm_hostname(deployment, realm, zone)?;

        let payload = self.payload(deployment, cell_id, realm).await?;
        self.record(
            deployment,
            REALM_CREATE,
            with_hostname(payload, &hostname),
            ActionSource::User {
                user_id: deployment.created_by.0,
            },
        )
        .await
    }

    pub async fn issue_administrator(
        &self,
        deployment: &Deployment,
        zone: &str,
        requested_by: crate::user::UserId,
        request: uuid::Uuid,
    ) -> Result<Action, CoreError> {
        let (cell_id, realm) = pooled(deployment)?;
        let hostname = realm_hostname(deployment, realm, zone)?;

        let mut payload = with_hostname(self.payload(deployment, cell_id, realm).await?, &hostname);
        payload["request_id"] = json!(request);
        self.record(
            deployment,
            REALM_ADMINISTRATOR_ISSUE,
            payload,
            ActionSource::User {
                user_id: requested_by.0,
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

fn realm_hostname(
    deployment: &Deployment,
    realm: &RealmName,
    zone: &str,
) -> Result<String, CoreError> {
    let hostname = hostname_for(&deployment.name.0, zone);
    if zone.is_empty() || hostname != format!("{realm}.{zone}") {
        return Err(CoreError::InternalError(format!(
            "deployment {} is not reachable as realm '{realm}' in zone '{zone}'",
            deployment.id.0
        )));
    }
    Ok(hostname)
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
    ) -> Result<Settlement, CoreError> {
        let Some(deployment) = self.deployments.get_by_id(deployment_id).await? else {
            return Ok(Settlement::Unchanged);
        };
        if !matches!(deployment.distribution, Distribution::Pooled { .. }) {
            return Ok(Settlement::Unchanged);
        }

        match (outcome, &deployment.status) {
            (DeploymentOutcome::Failed, DeploymentStatus::Failed) => self
                .cells
                .release_refused(deployment.id, reason.unwrap_or(UNSTATED_REFUSAL))
                .await
                .map(Settlement::released),
            (DeploymentOutcome::Deleted, DeploymentStatus::Deleted) => self
                .cells
                .release(deployment.id)
                .await
                .map(Settlement::released),
            (
                DeploymentOutcome::Running,
                DeploymentStatus::Deleting | DeploymentStatus::Deleted,
            ) => Ok(Settlement::RealmOutlivedTenant(Box::new(deployment))),
            _ => Ok(Settlement::Unchanged),
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
        cells::{CellStatus, ports::MockCellRepository},
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
        Cell::restore(
            CELL,
            Region::new("eu"),
            plane,
            INSTANCE,
            CellStatus::Open,
            200,
            0,
            Utc::now(),
        )
        .expect("an open cell")
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
    async fn an_administrator_issue_carries_the_realm_and_no_secret() {
        let (requests, recorded) = requests(Some(cell(PLANE)));
        let asker = crate::user::UserId(uuid::Uuid::from_u128(99));

        let action = requests
            .issue_administrator(
                &tenant(DeploymentStatus::Successful),
                "cells.example",
                asker,
                uuid::Uuid::from_u128(5),
            )
            .await
            .expect("recorded");

        assert_eq!(
            action.action_type,
            ActionType(REALM_ADMINISTRATOR_ISSUE.to_string())
        );
        assert_eq!(
            action.metadata.source,
            ActionSource::User { user_id: asker.0 }
        );
        assert_eq!(
            action.payload.data,
            json!({
                "deployment_id": TENANT.0,
                "cell": { "deployment_id": INSTANCE.0, "namespace": "cells-cell-one" },
                "realm": "acme",
                "hostname": "acme.cells.example",
                "request_id": uuid::Uuid::from_u128(5),
            })
        );
        assert_eq!(recorded_count(&recorded), 1);
    }

    #[tokio::test]
    async fn a_realm_create_names_the_cell_instance_the_realm_and_the_hostname() {
        let (requests, recorded) = requests(Some(cell(PLANE)));

        let action = requests
            .create(&tenant(DeploymentStatus::Pending), "cells.example")
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
            .create(&shared, "cells.example")
            .await
            .expect_err("refused");

        assert!(matches!(error, CoreError::InternalError(_)));
        assert_eq!(recorded_count(&recorded), 0);
    }

    #[tokio::test]
    async fn a_tenant_is_only_requested_as_its_realm_under_the_zone() {
        let mut renamed = tenant(DeploymentStatus::Pending);
        renamed.name = DeploymentName("Acme Corp".to_string());

        for (deployment, zone) in [
            (tenant(DeploymentStatus::Pending), ""),
            (renamed, "cells.example"),
        ] {
            let (requests, recorded) = requests(Some(cell(PLANE)));

            let result = requests.create(&deployment, zone).await;

            assert!(result.is_err(), "{zone}");
            assert_eq!(recorded_count(&recorded), 0);
        }
    }

    #[tokio::test]
    async fn an_unknown_cell_records_nothing() {
        let (requests, recorded) = requests(None);

        let error = requests
            .create(&tenant(DeploymentStatus::Pending), "cells.example")
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
            .create(&tenant(DeploymentStatus::Pending), "cells.example")
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

        let settled = outcomes(tenant(DeploymentStatus::Failed), cells)
            .settle(TENANT, DeploymentOutcome::Failed, Some("name refused"))
            .await
            .expect("settled");

        assert_eq!(settled, Settlement::Released);
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

        let settled = outcomes(tenant(DeploymentStatus::Deleted), cells)
            .settle(TENANT, DeploymentOutcome::Deleted, None)
            .await
            .expect("settled");

        assert_eq!(settled, Settlement::Released);
    }

    #[tokio::test]
    async fn nothing_is_released_unless_the_status_moved_to_match() {
        let cases = [
            (DeploymentStatus::Successful, DeploymentOutcome::Running),
            (DeploymentStatus::Pending, DeploymentOutcome::Running),
            (DeploymentStatus::Successful, DeploymentOutcome::Failed),
            (DeploymentStatus::Deleting, DeploymentOutcome::Deleted),
            (DeploymentStatus::Deleting, DeploymentOutcome::Failed),
        ];

        for (status, outcome) in cases {
            let settled = outcomes(tenant(status.clone()), MockCellRepository::new())
                .settle(TENANT, outcome, Some("x"))
                .await
                .expect("settled");

            assert_eq!(settled, Settlement::Unchanged, "{status:?} {outcome:?}");
        }
    }

    #[tokio::test]
    async fn a_realm_reported_serving_after_its_tenant_left_is_named_for_removal() {
        for status in [DeploymentStatus::Deleting, DeploymentStatus::Deleted] {
            let settled = outcomes(tenant(status.clone()), MockCellRepository::new())
                .settle(TENANT, DeploymentOutcome::Running, None)
                .await
                .expect("settled");

            assert!(
                matches!(&settled, Settlement::RealmOutlivedTenant(d) if d.id == TENANT),
                "{status:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_hand_off_releases_the_slot_with_its_own_reason() {
        let mut cells = MockCellRepository::new();
        cells
            .expect_release_refused()
            .withf(|deployment, reason| *deployment == TENANT && reason == HAND_OFF_FAILED)
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(true) }));

        let settled = outcomes(tenant(DeploymentStatus::Failed), cells)
            .settle(TENANT, DeploymentOutcome::Failed, Some(HAND_OFF_FAILED))
            .await
            .expect("settled");

        assert_eq!(settled, Settlement::Released);
    }

    #[tokio::test]
    async fn a_deployment_that_is_not_pooled_holds_no_slot() {
        let mut shared = deployment(TENANT, "acme", Distribution::Shared);
        shared.status = DeploymentStatus::Deleted;

        let settled = outcomes(shared, MockCellRepository::new())
            .settle(TENANT, DeploymentOutcome::Deleted, None)
            .await
            .expect("settled");

        assert_eq!(settled, Settlement::Unchanged);
    }
}
