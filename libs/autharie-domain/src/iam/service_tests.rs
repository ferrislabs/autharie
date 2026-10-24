use std::sync::{Arc, Mutex};

use autharie_auth::Identity;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::{
    CoreError,
    action::{Action, ports::MockActionRepository},
    agents::registry::SealingJwk,
    audit::{AuditEntry, ports::MockAuditRepository},
    dataplane::{
        entities::DataPlane,
        ports::MockDataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::MockDeploymentRepository,
    },
    organisation::OrganisationId,
    user::{User, UserId, ports::UserRepository},
    version::Version,
    webhooks::ports::KnownRealms,
};
use ports::{Clock, IamOutcomeService, IamPolicy, IamRepository, IamService};
use service::{IamOutcomeServiceImpl, IamServiceImpl};

const ORGANISATION: Uuid = Uuid::from_u128(1);
const DEPLOYMENT: Uuid = Uuid::from_u128(2);
const DATAPLANE: Uuid = Uuid::from_u128(3);
const TERM: &str = "alice-the-canary";

#[derive(Clone)]
struct FakeClock(Arc<Mutex<DateTime<Utc>>>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(
            Utc.with_ymd_and_hms(2026, 10, 24, 10, 0, 0).unwrap(),
        )))
    }

    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[derive(Clone)]
struct Row {
    request: IamRequest,
    envelope: Option<Envelope>,
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Vec<Row>>>);

impl Memory {
    fn rows(&self) -> Vec<Row> {
        self.0.lock().unwrap().clone()
    }
}

impl IamRepository for Memory {
    async fn open(&self, request: &IamRequest, max_outstanding: usize) -> Result<bool, CoreError> {
        let mut rows = self.0.lock().unwrap();
        let outstanding = rows
            .iter()
            .filter(|row| {
                row.request.deployment_id == request.deployment_id
                    && row.request.state != IamState::Delivered
                    && row.request.expires_at > request.created_at
            })
            .count();
        if outstanding >= max_outstanding {
            return Ok(false);
        }
        rows.push(Row {
            request: request.clone(),
            envelope: None,
        });
        Ok(true)
    }

    async fn get(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<IamRequest>, CoreError> {
        Ok(self
            .rows()
            .into_iter()
            .map(|row| row.request)
            .find(|request| {
                request.id == request_id
                    && request.deployment_id == deployment_id
                    && request.expires_at > now
            }))
    }

    async fn record_outcome(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        verdict: &IamVerdict,
        now: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut rows = self.0.lock().unwrap();
        let Some(row) = rows.iter_mut().find(|row| {
            row.request.id == request_id
                && row.request.deployment_id == deployment_id
                && row.request.state == IamState::Pending
                && row.request.expires_at > now
        }) else {
            return Ok(false);
        };
        match verdict {
            IamVerdict::Done(envelope) => {
                row.request.state = IamState::Done;
                row.envelope = Some(envelope.clone());
            }
            IamVerdict::Failed { message } => {
                row.request.state = IamState::Failed {
                    message: message.clone(),
                };
            }
        }
        Ok(true)
    }

    async fn take_envelope(
        &self,
        deployment_id: DeploymentId,
        request_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<Envelope>, CoreError> {
        let mut rows = self.0.lock().unwrap();
        let Some(row) = rows.iter_mut().find(|row| {
            row.request.id == request_id
                && row.request.deployment_id == deployment_id
                && row.request.state == IamState::Done
                && row.request.expires_at > now
        }) else {
            return Ok(None);
        };
        row.request.state = IamState::Delivered;
        Ok(row.envelope.take())
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64, CoreError> {
        let mut rows = self.0.lock().unwrap();
        let before = rows.len();
        rows.retain(|row| row.request.expires_at > now);
        Ok((before - rows.len()) as u64)
    }
}

struct Realms;

impl KnownRealms for Realms {
    async fn known_realms(&self, _: DeploymentId) -> Result<Vec<String>, CoreError> {
        Ok(vec!["master".to_string(), "customers".to_string()])
    }
}

struct Users;

impl UserRepository for Users {
    async fn upsert_by_email(&self, user: &User) -> Result<User, CoreError> {
        Ok(user.clone())
    }

    async fn find_by_sub(&self, sub: &str) -> Result<Option<User>, CoreError> {
        Ok(Some(User {
            id: UserId(Uuid::from_u128(4)),
            email: "member@example.com".to_string(),
            name: "member".to_string(),
            sub: sub.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn find_by_email(&self, _: &str) -> Result<Option<User>, CoreError> {
        Ok(None)
    }
}

#[derive(Clone, Copy)]
struct Policy {
    view: bool,
}

impl IamPolicy for Policy {
    async fn can_access_iam(
        &self,
        _: Identity,
        _: OrganisationId,
        _: IamAccess,
    ) -> Result<(), CoreError> {
        if self.view {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied {
                reason: "insufficient permissions".to_string(),
            })
        }
    }
}

fn caller(sub: &str) -> Identity {
    Identity::User(autharie_auth::User {
        id: sub.to_string(),
        username: sub.to_string(),
        email: None,
        name: None,
        roles: vec![],
        scopes: vec![],
        act: None,
    })
}

fn deployment(kind: DeploymentKind) -> Deployment {
    let at = Utc::now();
    Deployment {
        topology: Default::default(),
        id: DeploymentId(DEPLOYMENT),
        organisation_id: OrganisationId(ORGANISATION),
        dataplane_id: DataPlaneId(DATAPLANE),
        name: DeploymentName("auth".to_string()),
        kind,
        version: Version::new(26, 0, 1),
        status: DeploymentStatus::Successful,
        namespace: "production-auth".to_string(),
        environment: crate::deployments::environment::Environment::Development,
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
        network_access: crate::deployments::network::NetworkAccess::Open,
        last_verified_restore_at: None,
        last_restore_drill_seconds: None,
        log_shipping_enabled: false,
        iam_settings: Default::default(),
        distribution: Default::default(),
    }
}

fn jwk() -> SealingJwk {
    let mut modulus = vec![0xffu8; 256];
    modulus[255] = 0xfd;
    SealingJwk {
        kty: "RSA".to_string(),
        n: URL_SAFE_NO_PAD.encode(modulus),
        e: "AQAB".to_string(),
        alg: Some("RSA-OAEP-256".to_string()),
        d: None,
    }
}

fn envelope() -> Envelope {
    Envelope::parse(json!({
        "v": 1,
        "alg": "RSA-OAEP-256+A256GCM",
        "ek": URL_SAFE_NO_PAD.encode([1u8; 256]),
        "iv": URL_SAFE_NO_PAD.encode([2u8; 12]),
        "ct": URL_SAFE_NO_PAD.encode([3u8; 48]),
    }))
    .unwrap()
}

#[derive(Clone, Default)]
struct Written {
    actions: Arc<Mutex<Vec<Action>>>,
    audit: Arc<Mutex<Vec<AuditEntry>>>,
}

type Service = IamServiceImpl<
    MockDeploymentRepository,
    Memory,
    Realms,
    MockActionRepository,
    MockAuditRepository,
    Users,
    Policy,
    FakeClock,
>;

struct World {
    service: Service,
    memory: Memory,
    written: Written,
    clock: FakeClock,
}

fn world(kind: DeploymentKind, view: bool) -> World {
    let memory = Memory::default();
    let written = Written::default();
    let clock = FakeClock::new();

    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_get_by_id().returning(move |_| {
        let found = deployment(kind.clone());
        Box::pin(async move { Ok(Some(found)) })
    });

    let mut actions = MockActionRepository::new();
    let held = Arc::clone(&written.actions);
    actions.expect_append().returning(move |action| {
        held.lock().unwrap().push(action);
        Box::pin(async { Ok(()) })
    });

    let mut audit = MockAuditRepository::new();
    let trail = Arc::clone(&written.audit);
    audit.expect_append().returning(move |entry| {
        trail.lock().unwrap().push(entry);
        Box::pin(async { Ok(()) })
    });

    World {
        service: IamServiceImpl::new(
            deployments,
            memory.clone(),
            Realms,
            actions,
            audit,
            Users,
            Policy { view },
        )
        .with_clock(clock.clone()),
        memory,
        written,
        clock,
    }
}

fn ferriskey() -> World {
    world(DeploymentKind::Ferriskey, true)
}

fn input(realm: &str, operation: IamOperation) -> IamInput {
    IamInput::parse(realm, operation, jwk()).unwrap()
}

fn list() -> IamOperation {
    IamOperation::applications_list(None, None, None).unwrap()
}

async fn ask(world: &World, sub: &str, realm: &str) -> Result<Uuid, CoreError> {
    world
        .service
        .ask_iam(
            caller(sub),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input(realm, list()),
        )
        .await
}

async fn poll(world: &World, sub: &str, request_id: Uuid) -> Result<IamFetch, CoreError> {
    world
        .service
        .poll_iam(
            caller(sub),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            request_id,
        )
        .await
}

fn ingest(
    world: &World,
    speaking: Uuid,
) -> IamOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory, FakeClock> {
    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_get_by_id().returning(|_| {
        let found = deployment(DeploymentKind::Ferriskey);
        Box::pin(async move { Ok(Some(found)) })
    });
    let mut dataplanes = MockDataPlaneRepository::new();
    dataplanes
        .expect_find_by_herald_subject()
        .returning(move |_| {
            let mut dataplane = DataPlane::new(
                DataPlaneAllocation::Shared,
                Region::new("somewhere"),
                Capacity::new(1000, 1024, 10).unwrap(),
            );
            dataplane.id = DataPlaneId(speaking);
            Box::pin(async move { Ok(Some(dataplane)) })
        });

    IamOutcomeServiceImpl::new(deployments, dataplanes, world.memory.clone())
        .with_clock(world.clock.clone())
}

async fn report(world: &World, request_id: Uuid, verdict: IamVerdict) -> bool {
    ingest(world, DATAPLANE)
        .report_iam_outcome(
            caller("herald"),
            DeploymentId(DEPLOYMENT),
            IamOutcome {
                request_id,
                verdict,
            },
        )
        .await
        .expect("reported")
}

fn is_iam(error: CoreError) -> IamError {
    match error {
        CoreError::Iam(error) => error,
        other => panic!("not an iam error: {other}"),
    }
}

#[tokio::test]
async fn spec_iam_1_a_request_is_pending_then_collected_once_then_gone() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();

    assert_eq!(poll(&world, "alice", id).await.unwrap(), IamFetch::Pending);
    assert!(report(&world, id, IamVerdict::Done(envelope())).await);
    assert_eq!(
        poll(&world, "alice", id).await.unwrap(),
        IamFetch::Ready(envelope())
    );
    assert!(matches!(
        is_iam(poll(&world, "alice", id).await.unwrap_err()),
        IamError::Gone { .. }
    ));
}

#[tokio::test]
async fn spec_iam_2_what_is_stored_is_the_envelope_received_and_it_is_gone_once_delivered() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();
    report(&world, id, IamVerdict::Done(envelope())).await;

    assert_eq!(world.memory.rows()[0].envelope, Some(envelope()));

    poll(&world, "alice", id).await.unwrap();

    assert_eq!(world.memory.rows()[0].envelope, None);
    assert_eq!(world.memory.rows()[0].request.state, IamState::Delivered);
}

#[tokio::test]
async fn the_action_carries_the_normalised_operation_the_sealing_key_and_the_expiry() {
    let world = ferriskey();
    let operation = IamOperation::applications_list(None, Some(10), Some(TERM)).unwrap();
    let id = world
        .service
        .ask_iam(
            caller("alice"),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input("customers", operation),
        )
        .await
        .unwrap();

    let actions = world.written.actions.lock().unwrap().clone();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].action_type.0, IAM_ACTION_TYPE);
    assert_eq!(actions[0].version.0, 1);
    let data = &actions[0].payload.data;
    assert_eq!(data["deployment_id"], json!(DEPLOYMENT));
    assert_eq!(data["namespace"], "production-auth");
    assert_eq!(data["request_id"], json!(id));
    assert_eq!(data["realm"], "customers");
    assert_eq!(
        data["operation"],
        json!({ "type": "applications.list", "page": 1, "limit": 10, "search": TERM })
    );
    assert_eq!(data["sealing_key"]["alg"], "RSA-OAEP-256");
    assert_eq!(data["expires_at"], "2026-10-24T10:05:00Z");
}

#[tokio::test]
async fn spec_iam_7_a_member_who_cannot_view_is_refused_and_nothing_is_recorded() {
    let world = world(DeploymentKind::Ferriskey, false);

    let refused = ask(&world, "alice", "master").await.unwrap_err();

    assert!(matches!(refused, CoreError::PermissionDenied { .. }));
    assert!(world.memory.rows().is_empty());
    assert!(world.written.actions.lock().unwrap().is_empty());
    assert!(world.written.audit.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_iam_7_a_deployment_that_is_not_ferriskey_is_refused_and_nothing_is_recorded() {
    let world = world(DeploymentKind::Keycloak, true);

    let refused = ask(&world, "alice", "master").await.unwrap_err();

    assert_eq!(is_iam(refused), IamError::UnsupportedKind);
    assert!(world.memory.rows().is_empty());
    assert!(world.written.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_realm_the_deployment_does_not_have_is_refused_before_anything_is_recorded() {
    let world = ferriskey();

    let refused = ask(&world, "alice", "elsewhere").await.unwrap_err();

    assert!(matches!(is_iam(refused), IamError::Invalid { .. }));
    assert!(world.memory.rows().is_empty());
    assert!(world.written.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spec_iam_8_the_audit_entry_names_who_what_and_where_but_never_the_search_or_a_result() {
    let world = ferriskey();
    let operation = IamOperation::applications_list(None, None, Some(TERM)).unwrap();
    let id = world
        .service
        .ask_iam(
            caller("alice"),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input("master", operation),
        )
        .await
        .unwrap();
    report(&world, id, IamVerdict::Done(envelope())).await;
    poll(&world, "alice", id).await.unwrap();

    let trail = world.written.audit.lock().unwrap().clone();
    assert_eq!(trail.len(), 1);
    let entry = &trail[0];
    assert_eq!(entry.action.0, IAM_REQUESTED_AUDIT);
    assert_eq!(entry.organisation_id, OrganisationId(ORGANISATION));
    assert_eq!(
        entry.actor,
        crate::audit::AuditActor::User {
            user_id: Uuid::from_u128(4)
        }
    );
    assert_eq!(entry.target.id, DEPLOYMENT);
    assert_eq!(entry.recorded_at, world.memory.rows()[0].request.created_at);
    let change = entry.change.as_ref().expect("a change");
    assert_eq!(
        change.after(),
        &json!({ "request_id": id, "realm": "master", "operation": "applications.list" })
    );
    let printed = format!("{entry:?}");
    assert!(!printed.contains(TERM));
    assert!(!printed.contains("RSA-OAEP"));
    assert!(!printed.contains(&URL_SAFE_NO_PAD.encode([3u8; 48])));
}

#[tokio::test]
async fn spec_iam_9_an_unfetched_result_expires_after_five_minutes_and_is_purged() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();
    report(&world, id, IamVerdict::Done(envelope())).await;

    world
        .clock
        .advance(Duration::minutes(4) + Duration::seconds(59));
    assert!(poll(&world, "alice", id).await.is_ok());

    let late = ferriskey();
    let id = ask(&late, "alice", "master").await.unwrap();
    report(&late, id, IamVerdict::Done(envelope())).await;
    late.clock.advance(Duration::minutes(5));

    assert!(matches!(
        is_iam(poll(&late, "alice", id).await.unwrap_err()),
        IamError::NotFound { .. }
    ));
    assert!(!report(&late, id, IamVerdict::Done(envelope())).await);
    late.memory.purge_expired(late.clock.now()).await.unwrap();
    assert!(late.memory.rows().is_empty());
}

#[tokio::test]
async fn a_tombstone_answers_gone_until_the_request_expires_and_then_not_found() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();
    report(&world, id, IamVerdict::Done(envelope())).await;
    poll(&world, "alice", id).await.unwrap();

    world.clock.advance(Duration::minutes(3));
    assert!(matches!(
        is_iam(poll(&world, "alice", id).await.unwrap_err()),
        IamError::Gone { .. }
    ));

    world.clock.advance(Duration::minutes(2));
    assert!(matches!(
        is_iam(poll(&world, "alice", id).await.unwrap_err()),
        IamError::NotFound { .. }
    ));
}

#[tokio::test]
async fn a_pending_request_that_is_never_answered_expires_too() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();

    world.clock.advance(Duration::minutes(6));

    assert!(matches!(
        is_iam(poll(&world, "alice", id).await.unwrap_err()),
        IamError::NotFound { .. }
    ));
}

#[tokio::test]
async fn only_the_member_who_asked_can_collect_the_answer() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();
    report(&world, id, IamVerdict::Done(envelope())).await;

    assert!(matches!(
        is_iam(poll(&world, "mallory", id).await.unwrap_err()),
        IamError::NotFound { .. }
    ));
    assert_eq!(
        poll(&world, "alice", id).await.unwrap(),
        IamFetch::Ready(envelope())
    );
}

#[tokio::test]
async fn a_request_the_deployment_never_had_is_not_found() {
    let world = ferriskey();

    assert!(matches!(
        is_iam(poll(&world, "alice", Uuid::new_v4()).await.unwrap_err()),
        IamError::NotFound { .. }
    ));
}

#[tokio::test]
async fn a_failed_request_answers_its_message_each_time_it_is_asked() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();
    report(
        &world,
        id,
        IamVerdict::Failed {
            message: "the result is larger than 512 KiB".to_string(),
        },
    )
    .await;

    for _ in 0..2 {
        assert_eq!(
            is_iam(poll(&world, "alice", id).await.unwrap_err()),
            IamError::Failed {
                message: "the result is larger than 512 KiB".to_string()
            }
        );
    }
}

#[tokio::test]
async fn an_outcome_for_a_request_already_done_failed_or_delivered_is_ignored() {
    let world = ferriskey();
    let done = ask(&world, "alice", "master").await.unwrap();
    let failed = ask(&world, "alice", "master").await.unwrap();
    let delivered = ask(&world, "alice", "master").await.unwrap();

    assert!(report(&world, done, IamVerdict::Done(envelope())).await);
    assert!(
        report(
            &world,
            failed,
            IamVerdict::Failed {
                message: "no".to_string()
            }
        )
        .await
    );
    assert!(report(&world, delivered, IamVerdict::Done(envelope())).await);
    poll(&world, "alice", delivered).await.unwrap();

    for id in [done, failed, delivered] {
        assert!(!report(&world, id, IamVerdict::Done(envelope())).await);
        assert!(
            !report(
                &world,
                id,
                IamVerdict::Failed {
                    message: "again".to_string()
                }
            )
            .await
        );
    }
    assert_eq!(
        poll(&world, "alice", done).await.unwrap(),
        IamFetch::Ready(envelope())
    );
    assert!(poll(&world, "alice", failed).await.is_err());
}

#[tokio::test]
async fn an_outcome_for_an_unknown_request_is_ignored() {
    let world = ferriskey();

    assert!(!report(&world, Uuid::new_v4(), IamVerdict::Done(envelope())).await);
}

#[tokio::test]
async fn a_data_plane_may_only_report_for_its_own_deployments() {
    let world = ferriskey();
    let id = ask(&world, "alice", "master").await.unwrap();

    let refused = ingest(&world, Uuid::from_u128(99))
        .report_iam_outcome(
            caller("herald"),
            DeploymentId(DEPLOYMENT),
            IamOutcome {
                request_id: id,
                verdict: IamVerdict::Done(envelope()),
            },
        )
        .await;

    assert!(matches!(refused, Err(CoreError::PermissionDenied { .. })));
    assert_eq!(world.memory.rows()[0].request.state, IamState::Pending);
}

#[tokio::test]
async fn the_eleventh_outstanding_request_is_refused_and_a_collected_one_frees_a_slot() {
    let world = ferriskey();
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(ask(&world, "alice", "master").await.unwrap());
    }

    let refused = ask(&world, "alice", "master").await.unwrap_err();
    assert_eq!(is_iam(refused), IamError::TooManyRequests { max: 10 });
    assert_eq!(world.memory.rows().len(), 10);
    assert_eq!(world.written.actions.lock().unwrap().len(), 10);
    assert_eq!(world.written.audit.lock().unwrap().len(), 10);

    report(&world, ids[0], IamVerdict::Done(envelope())).await;
    poll(&world, "alice", ids[0]).await.unwrap();
    assert!(ask(&world, "alice", "master").await.is_ok());
    assert!(ask(&world, "alice", "master").await.is_err());

    world.clock.advance(Duration::minutes(6));
    assert!(ask(&world, "alice", "master").await.is_ok());
}

#[tokio::test]
async fn a_deployment_of_another_organisation_is_not_found() {
    let world = ferriskey();

    let refused = world
        .service
        .ask_iam(
            caller("alice"),
            OrganisationId(Uuid::from_u128(77)),
            DeploymentId(DEPLOYMENT),
            input("master", list()),
        )
        .await
        .unwrap_err();

    assert!(matches!(refused, CoreError::DeploymentNotFound { .. }));
}

#[test]
fn a_sealing_key_that_is_weak_or_not_rsa_never_becomes_an_input() {
    let mut weak = jwk();
    weak.n = URL_SAFE_NO_PAD.encode(vec![0xffu8; 255]);
    let mut ec = jwk();
    ec.kty = "EC".to_string();

    for key in [weak, ec] {
        assert!(IamInput::parse("master", list(), key).is_err());
    }
    assert!(IamInput::parse("bad realm", list(), jwk()).is_err());
}
