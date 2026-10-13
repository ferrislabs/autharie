use std::sync::Arc;

use tracing::{info, warn};

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::outcome::DeploymentOutcomeReport;
use crate::domain::entities::realm_payload::RealmPayloadV1;
use crate::domain::error::{GenesisError, RealmError};
use crate::domain::ports::{BoxFuture, EventHandler, OutcomePublisher, RealmPort, TenantRoutePort};

pub const REALM_CREATE: &str = "realm.create";
pub const REALM_DELETE: &str = "realm.delete";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RealmAction {
    Create,
    Delete,
}

pub struct RealmEventHandler<R, T> {
    action: RealmAction,
    realms: R,
    routes: T,
    outcomes: Arc<dyn OutcomePublisher>,
}

impl<R, T> RealmEventHandler<R, T>
where
    R: RealmPort,
    T: TenantRoutePort,
{
    pub fn create(realms: R, routes: T, outcomes: Arc<dyn OutcomePublisher>) -> Self {
        Self {
            action: RealmAction::Create,
            realms,
            routes,
            outcomes,
        }
    }

    pub fn delete(realms: R, routes: T, outcomes: Arc<dyn OutcomePublisher>) -> Self {
        Self {
            action: RealmAction::Delete,
            realms,
            routes,
            outcomes,
        }
    }

    async fn create_realm(&self, payload: RealmPayloadV1) -> Result<(), GenesisError> {
        let hostname = payload.hostname()?;
        let cell = payload.cell();
        info!(
            deployment_id = %payload.deployment_id,
            realm = %payload.realm,
            %hostname,
            cell = %cell.name,
            "creating a realm"
        );

        match self.realms.create_realm(&cell, &payload.realm).await {
            Ok(()) | Err(RealmError::AlreadyExists) => {}
            Err(RealmError::Refused { reason }) => {
                warn!(deployment_id = %payload.deployment_id, %reason, "the cell refused the realm");
                return self
                    .outcomes
                    .publish(DeploymentOutcomeReport::failed_because(
                        payload.deployment_id,
                        reason,
                    ))
                    .await;
            }
            Err(error @ (RealmError::NotFound | RealmError::CellUnreachable { .. })) => {
                return Err(retry(error));
            }
        }

        self.routes.apply(&payload.route(), hostname).await?;
        self.outcomes
            .publish(DeploymentOutcomeReport::serving(payload.deployment_id))
            .await
    }

    async fn delete_realm(&self, payload: RealmPayloadV1) -> Result<(), GenesisError> {
        let cell = payload.cell();
        info!(
            deployment_id = %payload.deployment_id,
            realm = %payload.realm,
            cell = %cell.name,
            "deleting a realm"
        );

        self.routes.delete(&payload.route()).await?;
        match self.realms.delete_realm(&cell, &payload.realm).await {
            Ok(()) | Err(RealmError::NotFound) => {}
            Err(error) => return Err(retry(error)),
        }

        self.outcomes
            .publish(DeploymentOutcomeReport::deleted(payload.deployment_id))
            .await
    }
}

fn retry(error: RealmError) -> GenesisError {
    GenesisError::Handler {
        message: error.to_string(),
    }
}

impl<R, T> EventHandler for RealmEventHandler<R, T>
where
    R: RealmPort,
    T: TenantRoutePort,
{
    fn routing_key(&self) -> &str {
        match self.action {
            RealmAction::Create => REALM_CREATE,
            RealmAction::Delete => REALM_DELETE,
        }
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = RealmPayloadV1::from_value(&event.payload)?;
            match self.action {
                RealmAction::Create => self.create_realm(payload).await,
                RealmAction::Delete => self.delete_realm(payload).await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;

    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domain::entities::identity_instance::IdentityInstanceRef;
    use crate::domain::entities::realm_payload::TenantRoute;

    const TENANT: Uuid = Uuid::from_u128(1);
    const CELL: Uuid = Uuid::from_u128(2);

    #[derive(Default)]
    struct FakeCell {
        realms: Mutex<BTreeSet<String>>,
        refusal: Option<String>,
        unreachable: bool,
        calls: Mutex<Vec<(IdentityInstanceRef, String)>>,
    }

    impl FakeCell {
        fn refusing(reason: &str) -> Self {
            Self {
                refusal: Some(reason.to_string()),
                ..Self::default()
            }
        }

        fn unreachable() -> Self {
            Self {
                unreachable: true,
                ..Self::default()
            }
        }

        fn realms(&self) -> BTreeSet<String> {
            self.realms.lock().expect("not poisoned").clone()
        }

        fn answer(&self, cell: &IdentityInstanceRef, realm: &str) -> Result<(), RealmError> {
            self.calls
                .lock()
                .expect("not poisoned")
                .push((cell.clone(), realm.to_string()));
            if self.unreachable {
                return Err(RealmError::CellUnreachable {
                    message: "timed out".to_string(),
                });
            }
            if let Some(reason) = &self.refusal {
                return Err(RealmError::Refused {
                    reason: reason.clone(),
                });
            }
            Ok(())
        }
    }

    impl RealmPort for &FakeCell {
        async fn create_realm(
            &self,
            cell: &IdentityInstanceRef,
            realm: &str,
        ) -> Result<(), RealmError> {
            self.answer(cell, realm)?;
            if self
                .realms
                .lock()
                .expect("not poisoned")
                .insert(realm.to_string())
            {
                Ok(())
            } else {
                Err(RealmError::AlreadyExists)
            }
        }

        async fn delete_realm(
            &self,
            cell: &IdentityInstanceRef,
            realm: &str,
        ) -> Result<(), RealmError> {
            self.answer(cell, realm)?;
            if self.realms.lock().expect("not poisoned").remove(realm) {
                Ok(())
            } else {
                Err(RealmError::NotFound)
            }
        }
    }

    #[derive(Default)]
    struct FakeRoutes {
        routes: Mutex<BTreeMap<String, (String, String)>>,
    }

    impl FakeRoutes {
        fn routes(&self) -> BTreeMap<String, (String, String)> {
            self.routes.lock().expect("not poisoned").clone()
        }
    }

    impl TenantRoutePort for &FakeRoutes {
        async fn apply(&self, route: &TenantRoute, hostname: &str) -> Result<(), GenesisError> {
            self.routes.lock().expect("not poisoned").insert(
                route.name.clone(),
                (route.cell.namespace.clone(), hostname.to_string()),
            );
            Ok(())
        }

        async fn delete(&self, route: &TenantRoute) -> Result<(), GenesisError> {
            self.routes
                .lock()
                .expect("not poisoned")
                .remove(&route.name);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Reports(Mutex<Vec<DeploymentOutcomeReport>>);

    impl Reports {
        fn all(&self) -> Vec<DeploymentOutcomeReport> {
            self.0.lock().expect("not poisoned").clone()
        }
    }

    impl OutcomePublisher for Reports {
        fn publish<'a>(
            &'a self,
            report: DeploymentOutcomeReport,
        ) -> BoxFuture<'a, Result<(), GenesisError>> {
            self.0.lock().expect("not poisoned").push(report);
            Box::pin(async { Ok(()) })
        }
    }

    fn event(routing_key: &str, hostname: Option<&str>) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::new_v4(),
            deployment_id: Some(TENANT),
            dataplane_id: Uuid::new_v4(),
            routing_key: routing_key.to_string(),
            version: 1,
            payload: json!({
                "deployment_id": TENANT,
                "cell": { "deployment_id": CELL, "namespace": "cells-one" },
                "realm": "acme",
                "hostname": hostname,
            }),
            occurred_at: Utc::now(),
        }
    }

    fn create_event() -> ActionEvent {
        event(REALM_CREATE, Some("acme.cells.example"))
    }

    fn route_name() -> String {
        format!("realm-{TENANT}")
    }

    /// @spec-fpr-1
    #[tokio::test]
    async fn a_created_realm_gets_a_route_for_its_hostname_and_reports_serving() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = RealmEventHandler::create(&cell, &routes, reports.clone());

        handler.handle(create_event()).await.expect("handled");

        assert_eq!(cell.realms(), BTreeSet::from(["acme".to_string()]));
        assert_eq!(
            cell.calls.lock().expect("not poisoned")[0].0,
            IdentityInstanceRef::for_deployment(CELL, "cells-one")
        );
        assert_eq!(
            routes.routes(),
            BTreeMap::from([(
                route_name(),
                ("cells-one".to_string(), "acme.cells.example".to_string())
            )])
        );
        assert_eq!(reports.all(), [DeploymentOutcomeReport::serving(TENANT)]);
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn creating_the_realm_twice_leaves_one_realm_and_succeeds_twice() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = RealmEventHandler::create(&cell, &routes, reports.clone());

        handler.handle(create_event()).await.expect("first");
        handler.handle(create_event()).await.expect("redelivered");

        assert_eq!(cell.realms().len(), 1);
        assert_eq!(routes.routes().len(), 1);
        assert_eq!(
            reports.all(),
            [
                DeploymentOutcomeReport::serving(TENANT),
                DeploymentOutcomeReport::serving(TENANT)
            ]
        );
    }

    /// @spec-fpr-7
    #[tokio::test]
    async fn a_refused_realm_reports_failed_with_the_reason_and_gets_no_route() {
        let (cell, routes, reports) = (
            FakeCell::refusing("FerrisKey answered 400: invalid realm"),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = RealmEventHandler::create(&cell, &routes, reports.clone());

        handler
            .handle(create_event())
            .await
            .expect("handled, not retried");

        assert!(routes.routes().is_empty());
        assert_eq!(
            reports.all(),
            [DeploymentOutcomeReport::failed_because(
                TENANT,
                "FerrisKey answered 400: invalid realm"
            )]
        );
    }

    #[tokio::test]
    async fn an_unreachable_cell_is_retried_and_reports_nothing() {
        let (cell, routes, reports) = (
            FakeCell::unreachable(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        let create = RealmEventHandler::create(&cell, &routes, reports.clone());
        let delete = RealmEventHandler::delete(&cell, &routes, reports.clone());

        assert!(matches!(
            create.handle(create_event()).await,
            Err(GenesisError::Handler { .. })
        ));
        assert!(matches!(
            delete.handle(event(REALM_DELETE, None)).await,
            Err(GenesisError::Handler { .. })
        ));
        assert!(routes.routes().is_empty());
        assert!(reports.all().is_empty());
    }

    /// @spec-fpr-8
    #[tokio::test]
    async fn deleting_removes_the_route_and_the_realm_and_a_repeat_still_reports_deleted() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        RealmEventHandler::create(&cell, &routes, Arc::new(Reports::default()))
            .handle(create_event())
            .await
            .expect("created");
        let delete = RealmEventHandler::delete(&cell, &routes, reports.clone());

        delete
            .handle(event(REALM_DELETE, None))
            .await
            .expect("deleted");
        delete
            .handle(event(REALM_DELETE, None))
            .await
            .expect("deleted again");

        assert!(cell.realms().is_empty());
        assert!(routes.routes().is_empty());
        assert_eq!(
            reports.all(),
            [
                DeploymentOutcomeReport::deleted(TENANT),
                DeploymentOutcomeReport::deleted(TENANT)
            ]
        );
    }

    #[tokio::test]
    async fn a_create_without_a_hostname_never_reaches_the_cell() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = RealmEventHandler::create(&cell, &routes, reports.clone());

        let result = handler.handle(event(REALM_CREATE, None)).await;

        assert!(matches!(result, Err(GenesisError::InvalidPayload { .. })));
        assert!(cell.calls.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn answers_to_the_realm_routing_keys() {
        let (cell, routes) = (FakeCell::default(), FakeRoutes::default());
        let reports: Arc<dyn OutcomePublisher> = Arc::new(Reports::default());

        assert_eq!(
            RealmEventHandler::create(&cell, &routes, Arc::clone(&reports)).routing_key(),
            "realm.create"
        );
        assert_eq!(
            RealmEventHandler::delete(&cell, &routes, reports).routing_key(),
            "realm.delete"
        );
    }
}
