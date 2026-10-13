use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::{error, info, warn};
use uuid::Uuid;

use crate::domain::entities::action_event::ActionEvent;
use crate::domain::entities::identity_instance::IdentityInstanceRef;
use crate::domain::entities::outcome::DeploymentOutcomeReport;
use crate::domain::entities::realm_payload::{RealmPayloadV1, TenantRoute};
use crate::domain::entities::tenant_access::TenantAccess;
use crate::domain::error::{GenesisError, RealmError};
use crate::domain::ports::{BoxFuture, EventHandler, OutcomePublisher, RealmPort, TenantRoutePort};

pub const REALM_CREATE: &str = "realm.create";
pub const REALM_DELETE: &str = "realm.delete";
pub const REALM_ADMINISTRATOR_ISSUE: &str = "realm.administrator.issue";
pub const MAX_REFUSED_DELETES: u32 = 8;

const NO_TENANT_ZONE: &str = "realm_no_tenant_zone";
const NOT_A_CELL: &str = "realm_not_a_cell";
const ROUTE_NOT_OWNED: &str = "realm_route_not_owned";
const DELETE_REFUSED: &str = "realm_delete_refused";
const REALM_CLAIMED: &str = "realm_claimed_by_another_deployment";
const REALM_NOT_CLAIMED: &str = "realm_not_claimed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RealmAction {
    Create,
    Delete,
    Issue,
}

pub struct RealmEventHandler<R, T> {
    action: RealmAction,
    realms: R,
    routes: T,
    outcomes: Arc<dyn OutcomePublisher>,
    zone: Option<String>,
    refused_deletes: Mutex<HashMap<Uuid, u32>>,
}

impl<R, T> RealmEventHandler<R, T>
where
    R: RealmPort,
    T: TenantRoutePort,
{
    pub fn create(
        realms: R,
        routes: T,
        outcomes: Arc<dyn OutcomePublisher>,
        zone: Option<String>,
    ) -> Self {
        Self::new(RealmAction::Create, realms, routes, outcomes, zone)
    }

    pub fn delete(
        realms: R,
        routes: T,
        outcomes: Arc<dyn OutcomePublisher>,
        zone: Option<String>,
    ) -> Self {
        Self::new(RealmAction::Delete, realms, routes, outcomes, zone)
    }

    pub fn issue(
        realms: R,
        routes: T,
        outcomes: Arc<dyn OutcomePublisher>,
        zone: Option<String>,
    ) -> Self {
        Self::new(RealmAction::Issue, realms, routes, outcomes, zone)
    }

    fn new(
        action: RealmAction,
        realms: R,
        routes: T,
        outcomes: Arc<dyn OutcomePublisher>,
        zone: Option<String>,
    ) -> Self {
        Self {
            action,
            realms,
            routes,
            outcomes,
            zone: zone.filter(|zone| !zone.is_empty()),
            refused_deletes: Mutex::default(),
        }
    }

    async fn create_realm(&self, payload: RealmPayloadV1, zone: &str) -> Result<(), GenesisError> {
        let hostname = payload.hostname_in(zone)?;
        let cell = payload.cell();
        info!(
            deployment_id = %payload.deployment_id,
            realm = %payload.realm,
            %hostname,
            cell = %cell.name,
            "creating a realm"
        );

        let route = payload.route();
        let claims = self.routes.claims(&route).await?;
        if let Some(holder) = claims.iter().find(|claim| !route.is_owned_by(claim)) {
            warn!(
                deployment_id = %payload.deployment_id,
                holder = holder.deployment.as_deref().unwrap_or("unknown"),
                realm = %payload.realm,
                "another deployment still claims this realm"
            );
            return Err(GenesisError::Handler {
                message: REALM_CLAIMED.to_string(),
            });
        }

        self.routes.apply(&route, hostname).await?;

        match self.realms.create_realm(&cell, &payload.realm).await {
            Ok(()) | Err(RealmError::AlreadyExists) => {}
            Err(RealmError::Refused { reason }) => {
                warn!(deployment_id = %payload.deployment_id, %reason, "the cell refused the realm");
                self.routes.delete(&route).await?;
                return self
                    .outcomes
                    .publish(DeploymentOutcomeReport::failed_because(
                        payload.deployment_id,
                        reason,
                    ))
                    .await;
            }
            Err(RealmError::NotACell) => {
                self.routes.delete(&route).await?;
                return refuse(payload.deployment_id, NOT_A_CELL);
            }
            Err(error @ (RealmError::NotFound | RealmError::CellUnreachable { .. })) => {
                return Err(retry(error));
            }
        }

        let deployment_id = payload.deployment_id;
        let access = TenantAccess::for_realm(&payload, hostname);
        match self.realms.open_access(&cell, &access).await {
            Ok(()) => {}
            Err(RealmError::NotACell) => return self.compensate(&route, &cell, &payload).await,
            Err(error) => return Err(retry(error)),
        }

        self.outcomes
            .publish(DeploymentOutcomeReport::serving(deployment_id))
            .await
    }

    async fn compensate(
        &self,
        route: &TenantRoute,
        cell: &IdentityInstanceRef,
        payload: &RealmPayloadV1,
    ) -> Result<(), GenesisError> {
        match self.realms.delete_realm(cell, &payload.realm).await {
            Ok(()) | Err(RealmError::NotFound) => self.routes.delete(route).await?,
            Err(_) => error!(
                deployment_id = %payload.deployment_id,
                "the cell lost its marker and its new realm could not be removed, the route is kept"
            ),
        }
        refuse(payload.deployment_id, NOT_A_CELL)
    }

    async fn issue_administrator(
        &self,
        payload: RealmPayloadV1,
        zone: &str,
    ) -> Result<(), GenesisError> {
        let hostname = payload.hostname_in(zone)?;
        let cell = payload.cell();
        let route = payload.route();
        info!(
            deployment_id = %payload.deployment_id,
            realm = %payload.realm,
            cell = %cell.name,
            "issuing the realm administrator"
        );

        match self.routes.owner(&route).await? {
            Some(owner) if route.is_owned_by(&owner) => {}
            _ => return refuse(payload.deployment_id, REALM_NOT_CLAIMED),
        }

        let access = TenantAccess::for_realm(&payload, hostname);
        match self.realms.issue_administrator(&cell, &access).await {
            Ok(administrator) => {
                self.outcomes
                    .publish(DeploymentOutcomeReport::administrator_issued(
                        payload.deployment_id,
                        administrator,
                    ))
                    .await
            }
            Err(RealmError::NotACell) => refuse(payload.deployment_id, NOT_A_CELL),
            Err(error) => Err(retry(error)),
        }
    }

    async fn delete_realm(
        &self,
        action_id: Uuid,
        payload: RealmPayloadV1,
    ) -> Result<(), GenesisError> {
        let cell = payload.cell();
        let route = payload.route();
        info!(
            deployment_id = %payload.deployment_id,
            realm = %payload.realm,
            cell = %cell.name,
            "deleting a realm"
        );

        match self.routes.owner(&route).await? {
            None => {
                info!(
                    deployment_id = %payload.deployment_id,
                    "the tenant has no route, its realm counts as deleted"
                );
                return self.report_deleted(payload.deployment_id).await;
            }
            Some(owner) if !route.is_owned_by(&owner) => {
                return refuse(payload.deployment_id, ROUTE_NOT_OWNED);
            }
            Some(_) => {}
        }

        match self.realms.delete_realm(&cell, &payload.realm).await {
            Ok(()) | Err(RealmError::NotFound) => {}
            Err(RealmError::NotACell) => return refuse(payload.deployment_id, NOT_A_CELL),
            Err(RealmError::Refused { .. }) => {
                return self.refused_delete(action_id, payload.deployment_id);
            }
            Err(error @ (RealmError::AlreadyExists | RealmError::CellUnreachable { .. })) => {
                return Err(retry(error));
            }
        }

        self.routes.delete(&route).await?;
        self.forget(action_id);
        self.report_deleted(payload.deployment_id).await
    }

    async fn report_deleted(&self, deployment_id: Uuid) -> Result<(), GenesisError> {
        self.outcomes
            .publish(DeploymentOutcomeReport::deleted(deployment_id))
            .await
    }

    fn refused_delete(&self, action_id: Uuid, deployment_id: Uuid) -> Result<(), GenesisError> {
        let attempts = {
            let mut refused = self
                .refused_deletes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let attempts = refused.entry(action_id).or_insert(0);
            *attempts = attempts.saturating_add(1);
            *attempts
        };

        if attempts < MAX_REFUSED_DELETES {
            return Err(GenesisError::Handler {
                message: format!("{DELETE_REFUSED} (attempt {attempts})"),
            });
        }

        self.forget(action_id);
        refuse(deployment_id, DELETE_REFUSED)
    }

    fn forget(&self, action_id: Uuid) {
        self.refused_deletes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&action_id);
    }
}

fn refuse(deployment_id: Uuid, code: &'static str) -> Result<(), GenesisError> {
    error!(%deployment_id, reason = code, "realm action refused and parked");
    Ok(())
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
            RealmAction::Issue => REALM_ADMINISTRATOR_ISSUE,
        }
    }

    fn handle<'a>(&'a self, event: ActionEvent) -> BoxFuture<'a, Result<(), GenesisError>> {
        Box::pin(async move {
            let payload = RealmPayloadV1::from_value(&event.payload)?;
            let Some(zone) = self.zone.as_deref() else {
                return refuse(payload.deployment_id, NO_TENANT_ZONE);
            };
            match self.action {
                RealmAction::Create => self.create_realm(payload, zone).await,
                RealmAction::Delete => self.delete_realm(event.action_id, payload).await,
                RealmAction::Issue => self.issue_administrator(payload, zone).await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::domain::entities::realm_payload::{RealmName, RouteOwner};
    use crate::domain::entities::tenant_access::InitialAdministrator;

    const TENANT: Uuid = Uuid::from_u128(1);
    const CELL: Uuid = Uuid::from_u128(2);
    const ZONE: &str = "cells.example";

    type Journal = Arc<Mutex<Vec<&'static str>>>;

    fn entries(journal: &Journal) -> Vec<&'static str> {
        journal.lock().expect("lock").clone()
    }

    #[derive(Default)]
    struct FakeCell {
        realms: Mutex<BTreeSet<String>>,
        failure: Option<RealmError>,
        calls: Mutex<Vec<(IdentityInstanceRef, String)>>,
        journal: Journal,
        access_failure: Option<RealmError>,
        accesses: Mutex<Vec<TenantAccess>>,
        delete_failure: Option<RealmError>,
        issue_failure: Option<RealmError>,
        issued: Mutex<Vec<InitialAdministrator>>,
    }

    impl FakeCell {
        fn failing_access(failure: RealmError) -> Self {
            Self {
                access_failure: Some(failure),
                ..Self::default()
            }
        }

        fn failing_issue(failure: RealmError) -> Self {
            Self {
                issue_failure: Some(failure),
                ..Self::default()
            }
        }

        fn issued(&self) -> Vec<InitialAdministrator> {
            self.issued.lock().expect("lock").clone()
        }

        fn accesses(&self) -> Vec<TenantAccess> {
            self.accesses.lock().expect("lock").clone()
        }

        fn failing(failure: RealmError) -> Self {
            Self {
                failure: Some(failure),
                ..Self::default()
            }
        }

        fn holding(realm: &str) -> Self {
            let cell = Self::default();
            cell.realms.lock().expect("lock").insert(realm.to_string());
            cell
        }

        fn realms(&self) -> BTreeSet<String> {
            self.realms.lock().expect("lock").clone()
        }

        fn calls(&self) -> usize {
            self.calls.lock().expect("lock").len()
        }

        fn answer(&self, cell: &IdentityInstanceRef, realm: &RealmName) -> Result<(), RealmError> {
            self.calls
                .lock()
                .expect("lock")
                .push((cell.clone(), realm.to_string()));
            match &self.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(()),
            }
        }
    }

    impl RealmPort for &FakeCell {
        async fn create_realm(
            &self,
            cell: &IdentityInstanceRef,
            realm: &RealmName,
        ) -> Result<(), RealmError> {
            self.journal.lock().expect("lock").push("create realm");
            self.answer(cell, realm)?;
            if self.realms.lock().expect("lock").insert(realm.to_string()) {
                Ok(())
            } else {
                Err(RealmError::AlreadyExists)
            }
        }

        async fn delete_realm(
            &self,
            cell: &IdentityInstanceRef,
            realm: &RealmName,
        ) -> Result<(), RealmError> {
            self.journal.lock().expect("lock").push("delete realm");
            self.answer(cell, realm)?;
            if let Some(failure) = &self.delete_failure {
                return Err(failure.clone());
            }
            if self.realms.lock().expect("lock").remove(realm.as_str()) {
                Ok(())
            } else {
                Err(RealmError::NotFound)
            }
        }

        async fn open_access(
            &self,
            _cell: &IdentityInstanceRef,
            access: &TenantAccess,
        ) -> Result<(), RealmError> {
            self.journal.lock().expect("lock").push("open access");
            self.accesses.lock().expect("lock").push(access.clone());
            match &self.access_failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(()),
            }
        }

        async fn issue_administrator(
            &self,
            _cell: &IdentityInstanceRef,
            _access: &TenantAccess,
        ) -> Result<InitialAdministrator, RealmError> {
            self.journal
                .lock()
                .expect("lock")
                .push("issue administrator");
            if let Some(failure) = &self.issue_failure {
                return Err(failure.clone());
            }
            let administrator = InitialAdministrator::generate("admin");
            self.issued
                .lock()
                .expect("lock")
                .push(administrator.clone());
            Ok(administrator)
        }
    }

    #[derive(Default)]
    struct FakeRoutes {
        routes: Mutex<BTreeMap<String, (String, RouteOwner)>>,
        journal: Journal,
    }

    impl FakeRoutes {
        fn with(name: &str, owner: RouteOwner) -> Self {
            let routes = Self::default();
            routes
                .routes
                .lock()
                .expect("lock")
                .insert(name.to_string(), ("acme.cells.example".to_string(), owner));
            routes
        }

        fn routes(&self) -> BTreeMap<String, (String, RouteOwner)> {
            self.routes.lock().expect("lock").clone()
        }
    }

    impl TenantRoutePort for &FakeRoutes {
        async fn apply(&self, route: &TenantRoute, hostname: &str) -> Result<(), GenesisError> {
            self.journal.lock().expect("lock").push("claim");
            self.routes.lock().expect("lock").insert(
                route.name.clone(),
                (
                    hostname.to_string(),
                    RouteOwner {
                        deployment: Some(route.deployment_id.to_string()),
                        realm: Some(route.realm.to_string()),
                    },
                ),
            );
            Ok(())
        }

        async fn owner(&self, route: &TenantRoute) -> Result<Option<RouteOwner>, GenesisError> {
            Ok(self
                .routes
                .lock()
                .expect("lock")
                .get(&route.name)
                .map(|(_, owner)| owner.clone()))
        }

        async fn claims(&self, route: &TenantRoute) -> Result<Vec<RouteOwner>, GenesisError> {
            Ok(self
                .routes
                .lock()
                .expect("lock")
                .values()
                .filter(|(_, owner)| owner.realm.as_deref() == Some(route.realm.as_str()))
                .map(|(_, owner)| owner.clone())
                .collect())
        }

        async fn delete(&self, route: &TenantRoute) -> Result<(), GenesisError> {
            self.journal.lock().expect("lock").push("release");
            self.routes.lock().expect("lock").remove(&route.name);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Reports {
        sent: Mutex<Vec<DeploymentOutcomeReport>>,
        failing_first: Mutex<bool>,
    }

    impl Reports {
        fn all(&self) -> Vec<DeploymentOutcomeReport> {
            self.sent.lock().expect("lock").clone()
        }

        fn failing_once() -> Self {
            Self {
                failing_first: Mutex::new(true),
                ..Self::default()
            }
        }
    }

    impl OutcomePublisher for Reports {
        fn publish<'a>(
            &'a self,
            report: DeploymentOutcomeReport,
        ) -> BoxFuture<'a, Result<(), GenesisError>> {
            let failing = std::mem::take(&mut *self.failing_first.lock().expect("lock"));
            if failing {
                return Box::pin(async {
                    Err(GenesisError::MessageBus {
                        message: "the broker is away".to_string(),
                    })
                });
            }
            self.sent.lock().expect("lock").push(report);
            Box::pin(async { Ok(()) })
        }
    }

    fn event(routing_key: &str, hostname: Option<&str>) -> ActionEvent {
        ActionEvent {
            action_id: Uuid::from_u128(42),
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

    fn issue_event() -> ActionEvent {
        event(REALM_ADMINISTRATOR_ISSUE, Some("acme.cells.example"))
    }

    fn delete_event() -> ActionEvent {
        event(REALM_DELETE, None)
    }

    fn route_name() -> String {
        format!("realm-{TENANT}")
    }

    fn owner(deployment: Uuid, realm: &str) -> RouteOwner {
        RouteOwner {
            deployment: Some(deployment.to_string()),
            realm: Some(realm.to_string()),
        }
    }

    fn zone() -> Option<String> {
        Some(ZONE.to_string())
    }

    type Handler<'a> = RealmEventHandler<&'a FakeCell, &'a FakeRoutes>;

    fn creating<'a>(
        cell: &'a FakeCell,
        routes: &'a FakeRoutes,
        reports: &Arc<Reports>,
    ) -> Handler<'a> {
        RealmEventHandler::create(cell, routes, reports.clone(), zone())
    }

    fn issuing<'a>(
        cell: &'a FakeCell,
        routes: &'a FakeRoutes,
        reports: &Arc<Reports>,
    ) -> Handler<'a> {
        RealmEventHandler::issue(cell, routes, reports.clone(), zone())
    }

    fn deleting<'a>(
        cell: &'a FakeCell,
        routes: &'a FakeRoutes,
        reports: &Arc<Reports>,
    ) -> Handler<'a> {
        RealmEventHandler::delete(cell, routes, reports.clone(), zone())
    }

    /// @spec-fpr-1
    #[tokio::test]
    async fn a_created_realm_gets_a_labelled_route_for_its_hostname_and_reports_serving() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("handled");

        assert_eq!(cell.realms(), BTreeSet::from(["acme".to_string()]));
        assert_eq!(
            cell.calls.lock().expect("lock")[0].0,
            IdentityInstanceRef::for_deployment(CELL, "cells-one")
        );
        assert_eq!(
            routes.routes(),
            BTreeMap::from([(
                route_name(),
                ("acme.cells.example".to_string(), owner(TENANT, "acme"))
            )])
        );
        assert_eq!(reports.all(), [DeploymentOutcomeReport::serving(TENANT)]);
    }

    /// @spec-fpr-6
    #[tokio::test]
    async fn creating_the_realm_twice_leaves_one_realm_and_reports_serving_twice() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = creating(&cell, &routes, &reports);

        handler.handle(create_event()).await.expect("first");
        handler.handle(create_event()).await.expect("redelivered");

        assert_eq!(cell.realms().len(), 1);
        assert_eq!(routes.routes().len(), 1);
        assert_eq!(cell.accesses().len(), 2);
        assert!(cell.issued().is_empty());
        assert_eq!(
            reports.all(),
            [
                DeploymentOutcomeReport::serving(TENANT),
                DeploymentOutcomeReport::serving(TENANT)
            ]
        );
    }

    #[tokio::test]
    async fn a_create_never_issues_a_password_and_publishes_no_administrator() {
        let journal = Journal::default();
        let cell = FakeCell {
            journal: Arc::clone(&journal),
            ..FakeCell::default()
        };
        let (routes, reports) = (FakeRoutes::default(), Arc::new(Reports::default()));

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("created");

        assert!(!entries(&journal).contains(&"issue administrator"));
        assert!(cell.issued().is_empty());
        assert!(
            reports
                .all()
                .iter()
                .all(|report| report.administrator.is_none())
        );
    }

    #[tokio::test]
    async fn a_publish_that_failed_once_is_published_on_redelivery() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::failing_once()),
        );
        let handler = creating(&cell, &routes, &reports);

        let first = handler.handle(create_event()).await;
        assert!(matches!(first, Err(GenesisError::MessageBus { .. })));
        assert!(reports.all().is_empty());
        handler.handle(create_event()).await.expect("redelivered");

        assert_eq!(reports.all(), [DeploymentOutcomeReport::serving(TENANT)]);
    }

    #[tokio::test]
    async fn an_instance_that_loses_its_marker_after_the_realm_is_compensated() {
        let journal = Journal::default();
        let cell = FakeCell {
            journal: Arc::clone(&journal),
            access_failure: Some(RealmError::NotACell),
            ..FakeCell::default()
        };
        let routes = FakeRoutes {
            journal: Arc::clone(&journal),
            ..FakeRoutes::default()
        };
        let reports = Arc::new(Reports::default());

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("parked");

        assert!(cell.realms().is_empty());
        assert!(routes.routes().is_empty());
        assert_eq!(
            entries(&journal),
            [
                "claim",
                "create realm",
                "open access",
                "delete realm",
                "release"
            ]
        );
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn a_failed_compensation_keeps_the_route_and_publishes_nothing() {
        let cell = FakeCell {
            access_failure: Some(RealmError::NotACell),
            delete_failure: Some(RealmError::CellUnreachable {
                message: "the cell is unreachable".to_string(),
            }),
            ..FakeCell::default()
        };
        let (routes, reports) = (FakeRoutes::default(), Arc::new(Reports::default()));

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("parked");

        assert_eq!(routes.routes().len(), 1);
        assert_eq!(cell.realms().len(), 1);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn a_compensation_that_finds_no_realm_still_releases_the_route() {
        let cell = FakeCell {
            access_failure: Some(RealmError::NotACell),
            delete_failure: Some(RealmError::NotFound),
            ..FakeCell::default()
        };
        let (routes, reports) = (FakeRoutes::default(), Arc::new(Reports::default()));

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("parked");

        assert!(routes.routes().is_empty());
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn an_issue_on_an_owned_route_publishes_a_fresh_administrator() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        issuing(&cell, &routes, &reports)
            .handle(issue_event())
            .await
            .expect("issued");

        let issued = cell.issued();
        assert_eq!(issued.len(), 1);
        assert_eq!(
            reports.all(),
            [DeploymentOutcomeReport::administrator_issued(
                TENANT,
                issued[0].clone()
            )]
        );
        assert_eq!(reports.all()[0].outcome, "administrator_issued");
    }

    #[tokio::test]
    async fn two_issues_produce_two_different_passwords() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );
        let handler = issuing(&cell, &routes, &reports);

        handler.handle(issue_event()).await.expect("first");
        handler.handle(issue_event()).await.expect("second");

        let issued = cell.issued();
        assert_eq!(issued.len(), 2);
        assert_ne!(issued[0].password(), issued[1].password());
        assert_eq!(reports.all().len(), 2);
    }

    #[tokio::test]
    async fn an_issue_without_an_owned_route_makes_no_realm_call_and_no_outcome() {
        for routes in [
            FakeRoutes::default(),
            FakeRoutes::with(&route_name(), owner(CELL, "acme")),
            FakeRoutes::with(&route_name(), owner(TENANT, "other")),
            FakeRoutes::with(&route_name(), RouteOwner::default()),
        ] {
            let (cell, reports) = (FakeCell::default(), Arc::new(Reports::default()));

            issuing(&cell, &routes, &reports)
                .handle(issue_event())
                .await
                .expect("parked");

            assert!(cell.issued().is_empty());
            assert_eq!(cell.calls(), 0);
            assert!(reports.all().is_empty());
        }
    }

    #[tokio::test]
    async fn an_issue_error_is_retried_and_publishes_nothing() {
        let (cell, routes, reports) = (
            FakeCell::failing_issue(RealmError::CellUnreachable {
                message: "the cell is unreachable".to_string(),
            }),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        let result = issuing(&cell, &routes, &reports)
            .handle(issue_event())
            .await;

        assert!(matches!(result, Err(GenesisError::Handler { .. })));
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn an_issue_on_a_lost_cell_is_parked() {
        let (cell, routes, reports) = (
            FakeCell::failing_issue(RealmError::NotACell),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        issuing(&cell, &routes, &reports)
            .handle(issue_event())
            .await
            .expect("parked");

        assert!(reports.all().is_empty());
        assert_eq!(routes.routes().len(), 1);
    }

    #[tokio::test]
    async fn an_issue_error_never_carries_a_password_or_a_server_body() {
        let (cell, routes, reports) = (
            FakeCell::failing_issue(RealmError::CellUnreachable {
                message: "the password reset answered 500".to_string(),
            }),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        let error = issuing(&cell, &routes, &reports)
            .handle(issue_event())
            .await
            .expect_err("retried");

        let shown = format!("{error} {error:?}");
        assert!(shown.contains("the password reset answered 500"));
        assert!(!shown.contains("value"));
        assert!(cell.issued().is_empty());
    }

    #[tokio::test]
    async fn an_issue_outside_the_tenant_zone_never_reaches_the_cell() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );
        let handler = issuing(&cell, &routes, &reports);

        for hostname in [None, Some("acme.elsewhere.example")] {
            let result = handler
                .handle(event(REALM_ADMINISTRATOR_ISSUE, hostname))
                .await;
            assert!(matches!(result, Err(GenesisError::InvalidPayload { .. })));
        }
        assert_eq!(cell.calls(), 0);
        assert!(cell.issued().is_empty());
    }

    #[tokio::test]
    async fn an_access_error_is_retried_and_publishes_nothing() {
        let (cell, routes, reports) = (
            FakeCell::failing_access(RealmError::CellUnreachable {
                message: "step answered 500".to_string(),
            }),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        let result = creating(&cell, &routes, &reports)
            .handle(create_event())
            .await;

        assert!(matches!(result, Err(GenesisError::Handler { .. })));
        assert_eq!(cell.accesses().len(), 1);
        assert_eq!(routes.routes().len(), 1);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn access_is_opened_for_the_realm_on_its_hostname_after_the_realm_exists() {
        let journal = Journal::default();
        let cell = FakeCell {
            journal: Arc::clone(&journal),
            ..FakeCell::default()
        };
        let (routes, reports) = (FakeRoutes::default(), Arc::new(Reports::default()));

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("created");

        assert_eq!(entries(&journal), ["create realm", "open access"]);
        let access = &cell.accesses()[0];
        assert_eq!(access.realm.as_str(), "acme");
        assert_eq!(access.administrator_id, TENANT);
        assert_eq!(
            access.console_callback,
            "https://acme.cells.example/realms/acme/authentication/callback"
        );
    }

    /// @spec-fpr-7
    #[tokio::test]
    async fn a_refused_realm_reports_failed_with_the_reason_and_gets_no_route() {
        let (cell, routes, reports) = (
            FakeCell::failing(RealmError::Refused {
                reason: "realm refused by the cell (400)".to_string(),
            }),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("handled, not retried");

        assert!(routes.routes().is_empty());
        assert_eq!(
            reports.all(),
            [DeploymentOutcomeReport::failed_because(
                TENANT,
                "realm refused by the cell (400)"
            )]
        );
    }

    #[tokio::test]
    async fn an_unreachable_cell_is_retried_and_reports_nothing() {
        let unreachable = RealmError::CellUnreachable {
            message: "FerrisKey is unreachable".to_string(),
        };
        let (cell, routes, reports) = (
            FakeCell::failing(unreachable),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        let created = creating(&cell, &routes, &reports)
            .handle(create_event())
            .await;
        let deleted = deleting(&cell, &routes, &reports)
            .handle(delete_event())
            .await;

        assert!(matches!(created, Err(GenesisError::Handler { .. })));
        assert!(matches!(deleted, Err(GenesisError::Handler { .. })));
        assert_eq!(routes.routes().len(), 1);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn an_instance_that_is_not_a_cell_is_refused_without_retry_or_outcome() {
        let (cell, routes, reports) = (
            FakeCell::failing(RealmError::NotACell),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("parked");
        assert!(routes.routes().is_empty());

        let routes = FakeRoutes::with(&route_name(), owner(TENANT, "acme"));
        deleting(&cell, &routes, &reports)
            .handle(delete_event())
            .await
            .expect("parked");

        assert!(reports.all().is_empty());
        assert_eq!(routes.routes().len(), 1);
    }

    #[tokio::test]
    async fn the_claim_is_taken_before_the_realm_and_released_after_it() {
        let journal = Journal::default();
        let cell = FakeCell {
            journal: Arc::clone(&journal),
            ..FakeCell::default()
        };
        let routes = FakeRoutes {
            journal: Arc::clone(&journal),
            ..FakeRoutes::default()
        };
        let reports = Arc::new(Reports::default());

        creating(&cell, &routes, &reports)
            .handle(create_event())
            .await
            .expect("created");
        deleting(&cell, &routes, &reports)
            .handle(delete_event())
            .await
            .expect("deleted");

        assert_eq!(
            entries(&journal),
            [
                "claim",
                "create realm",
                "open access",
                "delete realm",
                "release"
            ]
        );
    }

    #[tokio::test]
    async fn a_stale_create_leaves_a_realm_another_deployment_claims_and_is_retried() {
        let other = Uuid::from_u128(77);
        let (cell, routes, reports) = (
            FakeCell::holding("acme"),
            FakeRoutes::with(&format!("realm-{other}"), owner(other, "acme")),
            Arc::new(Reports::default()),
        );

        let result = creating(&cell, &routes, &reports)
            .handle(create_event())
            .await;

        assert!(matches!(result, Err(GenesisError::Handler { .. })));
        assert_eq!(cell.calls(), 0);
        assert_eq!(cell.realms(), BTreeSet::from(["acme".to_string()]));
        assert_eq!(routes.routes().len(), 1);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn a_compensating_delete_behind_another_claim_never_reaches_the_realm() {
        let other = Uuid::from_u128(77);
        let (cell, routes, reports) = (
            FakeCell::holding("acme"),
            FakeRoutes::with(&format!("realm-{other}"), owner(other, "acme")),
            Arc::new(Reports::default()),
        );

        deleting(&cell, &routes, &reports)
            .handle(delete_event())
            .await
            .expect("deleted");

        assert_eq!(cell.calls(), 0);
        assert_eq!(cell.realms(), BTreeSet::from(["acme".to_string()]));
        assert_eq!(routes.routes().len(), 1);
        assert_eq!(reports.all(), [DeploymentOutcomeReport::deleted(TENANT)]);
    }

    /// @spec-fpr-8
    #[tokio::test]
    async fn deleting_removes_the_realm_then_the_route_and_a_repeat_still_reports_deleted() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        creating(&cell, &routes, &Arc::new(Reports::default()))
            .handle(create_event())
            .await
            .expect("created");
        let delete = deleting(&cell, &routes, &reports);

        delete.handle(delete_event()).await.expect("deleted");
        delete.handle(delete_event()).await.expect("deleted again");

        assert!(cell.realms().is_empty());
        assert!(routes.routes().is_empty());
        assert_eq!(cell.calls(), 2);
        assert_eq!(
            reports.all(),
            [
                DeploymentOutcomeReport::deleted(TENANT),
                DeploymentOutcomeReport::deleted(TENANT)
            ]
        );
    }

    #[tokio::test]
    async fn a_delete_before_the_create_applied_anything_never_calls_the_cell() {
        let (cell, routes, reports) = (
            FakeCell::holding("acme"),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        deleting(&cell, &routes, &reports)
            .handle(delete_event())
            .await
            .expect("handled");

        assert_eq!(cell.calls(), 0);
        assert_eq!(cell.realms().len(), 1);
        assert_eq!(reports.all(), [DeploymentOutcomeReport::deleted(TENANT)]);
    }

    #[tokio::test]
    async fn a_route_that_belongs_to_another_tenant_or_realm_stops_the_delete() {
        for stranger in [
            owner(CELL, "acme"),
            owner(TENANT, "other"),
            RouteOwner::default(),
        ] {
            let (cell, routes, reports) = (
                FakeCell::holding("acme"),
                FakeRoutes::with(&route_name(), stranger.clone()),
                Arc::new(Reports::default()),
            );

            deleting(&cell, &routes, &reports)
                .handle(delete_event())
                .await
                .expect("parked");

            assert_eq!(cell.calls(), 0, "{stranger:?}");
            assert_eq!(routes.routes().len(), 1);
            assert!(reports.all().is_empty());
        }
    }

    #[tokio::test]
    async fn a_refused_delete_is_retried_a_bounded_number_of_times_then_parked_holding_the_slot() {
        let (cell, routes, reports) = (
            FakeCell::failing(RealmError::Refused {
                reason: "realm refused by the cell (400)".to_string(),
            }),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );
        let delete = deleting(&cell, &routes, &reports);

        for attempt in 1..MAX_REFUSED_DELETES {
            let result = delete.handle(delete_event()).await;
            assert!(
                matches!(result, Err(GenesisError::Handler { .. })),
                "{attempt}"
            );
        }
        delete.handle(delete_event()).await.expect("parked");

        assert_eq!(cell.calls(), MAX_REFUSED_DELETES as usize);
        assert_eq!(routes.routes().len(), 1);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn without_a_tenant_zone_no_realm_action_reaches_the_cell() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::with(&route_name(), owner(TENANT, "acme")),
            Arc::new(Reports::default()),
        );

        for handler in [
            RealmEventHandler::create(&cell, &routes, reports.clone(), None),
            RealmEventHandler::delete(&cell, &routes, reports.clone(), Some(String::new())),
            RealmEventHandler::issue(&cell, &routes, reports.clone(), None),
        ] {
            let key = handler.routing_key().to_string();
            let event = event(&key, Some("acme.cells.example"));
            handler.handle(event).await.expect("parked");
        }

        assert_eq!(cell.calls(), 0);
        assert!(reports.all().is_empty());
    }

    #[tokio::test]
    async fn a_hostname_outside_the_tenant_zone_never_reaches_the_cell() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );
        let handler = creating(&cell, &routes, &reports);

        for hostname in [None, Some("acme.elsewhere.example")] {
            let result = handler.handle(event(REALM_CREATE, hostname)).await;
            assert!(matches!(result, Err(GenesisError::InvalidPayload { .. })));
        }
        assert_eq!(cell.calls(), 0);
    }

    #[test]
    fn answers_to_the_realm_routing_keys() {
        let (cell, routes, reports) = (
            FakeCell::default(),
            FakeRoutes::default(),
            Arc::new(Reports::default()),
        );

        assert_eq!(
            creating(&cell, &routes, &reports).routing_key(),
            "realm.create"
        );
        assert_eq!(
            deleting(&cell, &routes, &reports).routing_key(),
            "realm.delete"
        );
        assert_eq!(
            issuing(&cell, &routes, &reports).routing_key(),
            "realm.administrator.issue"
        );
    }
}
