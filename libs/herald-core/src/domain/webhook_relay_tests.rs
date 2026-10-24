use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::entities::action::Action;
use crate::domain::entities::dataplane::DataPlaneId;
use crate::domain::entities::deployment::{Deployment, DeploymentId, DeploymentKind};
use crate::domain::entities::logs::OrganisationId;
use crate::domain::entities::outcome::{DeploymentOutcomeReport, InboundOutcome};
use crate::domain::entities::shard::ShardConfig;
use crate::domain::error::HeraldError;
use crate::domain::log_index::LogIndexDocument;
use crate::domain::ports::{
    HeartbeatOutcome, HeraldService, LogIndexSink, MockControlPlaneRepository,
    MockInstanceStateSource, MockLogIndexSink, MockMessageBusRepository,
    MockOutcomeInboxRepository, MockPodLogSource, MockUsageSource, MockWebhookDeliverySource,
    WebhookDeliverySource,
};
use crate::domain::services::HeraldServiceImpl;

type Asked = Arc<StdMutex<Vec<(String, String)>>>;
type Shipped = Arc<StdMutex<Vec<(OrganisationId, Vec<Value>)>>>;

const SECRET: &str = "whsec_super_secret_value";

type Service = HeraldServiceImpl<
    MockControlPlaneRepository,
    MockMessageBusRepository,
    MockOutcomeInboxRepository,
    MockUsageSource,
    MockPodLogSource,
    MockInstanceStateSource,
>;

fn webhooks_payload(deployment_id: Uuid) -> Value {
    json!({
        "deployment_id": deployment_id,
        "kind": "webhooks",
        "revision": 4,
        "webhooks": [{ "id": "w1", "phase": "Active", "secret_version": 1 }],
        "secrets": [{ "id": "w1", "version": 1, "secret": SECRET }],
        "realms": ["master"],
        "requests": []
    })
}

fn deployment(kind: Option<DeploymentKind>, namespace: Option<&str>) -> Deployment {
    Deployment {
        id: DeploymentId::new("11111111-1111-1111-1111-111111111111"),
        dataplane_id: DataPlaneId::new("cccccccc-cccc-cccc-cccc-cccccccccccc"),
        organisation_id: OrganisationId::new("dddddddd-dddd-dddd-dddd-dddddddddddd"),
        name: "acme".to_string(),
        kind,
        namespace: namespace.map(str::to_string),
        log_shipping_enabled: false,
        transition: None,
        changed_at: None,
    }
}

fn control_plane(deployments: Vec<Deployment>) -> MockControlPlaneRepository {
    let mut control_plane = MockControlPlaneRepository::new();
    control_plane
        .expect_send_heartbeat()
        .returning(|_, _| Box::pin(async { Ok(HeartbeatOutcome::default()) }));
    control_plane.expect_list_deployments().returning(move |_| {
        let deployments = deployments.clone();
        Box::pin(async move { Ok(deployments) })
    });
    control_plane
        .expect_claim_actions()
        .returning(|_, _| Box::pin(async { Ok(Vec::<Action>::new()) }));
    control_plane
}

fn service(
    control_plane: MockControlPlaneRepository,
    outcomes: MockOutcomeInboxRepository,
) -> Service {
    HeraldServiceImpl::new(
        Arc::new(control_plane),
        Arc::new(MockMessageBusRepository::new()),
        Arc::new(outcomes),
        Arc::new(MockUsageSource::new()),
        Arc::new(MockPodLogSource::new()),
        Arc::new(MockInstanceStateSource::new()),
        DataPlaneId::new("cccccccc-cccc-cccc-cccc-cccccccccccc"),
        ShardConfig::new(0, 1),
    )
}

fn queued(outcomes: Vec<InboundOutcome>) -> MockOutcomeInboxRepository {
    let mut inbox = MockOutcomeInboxRepository::new();
    let mut queued = Some(outcomes);
    inbox.expect_drain().returning(move |_| {
        let drained = queued.take().unwrap_or_default();
        Box::pin(async move { Ok(drained) })
    });
    inbox
}

#[derive(Clone, Default)]
struct Captured(Arc<StdMutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn capture_logs(captured: &Captured) -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish(),
    )
}

#[tokio::test]
async fn a_webhooks_outcome_is_forwarded_verbatim_and_never_logged() {
    let captured = Captured::default();
    let _logs = capture_logs(&captured);
    let deployment_id = Uuid::new_v4();
    let payload = webhooks_payload(deployment_id);
    let forwarded: Arc<StdMutex<Vec<Value>>> = Arc::default();

    let mut control_plane = control_plane(Vec::new());
    control_plane.expect_report_outcome().never();
    control_plane.expect_report_drill_outcome().never();
    let seen = Arc::clone(&forwarded);
    control_plane
        .expect_report_webhooks_outcome()
        .times(1)
        .returning(move |_, outcome| {
            assert_eq!(outcome.deployment_id(), deployment_id);
            seen.lock().unwrap().push(outcome.body().clone());
            Box::pin(async { Ok(()) })
        });

    let outcome = InboundOutcome::parse(payload.to_string().as_bytes()).unwrap();
    service(control_plane, queued(vec![outcome]))
        .sync_all_deployments()
        .await
        .expect("cycle succeeds");

    assert_eq!(*forwarded.lock().unwrap(), vec![payload]);
    let logs = captured.text();
    assert!(logs.contains(&deployment_id.to_string()), "{logs}");
    assert!(logs.contains("webhooks"), "{logs}");
    assert!(!logs.contains(SECRET), "{logs}");
}

#[tokio::test]
async fn a_failed_webhooks_relay_does_not_log_the_payload_either() {
    let captured = Captured::default();
    let _logs = capture_logs(&captured);
    let deployment_id = Uuid::new_v4();

    let mut control_plane = control_plane(Vec::new());
    control_plane
        .expect_report_webhooks_outcome()
        .times(1)
        .returning(|_, _| {
            Box::pin(async {
                Err(HeraldError::ControlPlane {
                    message: "report_webhooks_outcome failed with status 503".to_string(),
                })
            })
        });

    let outcome =
        InboundOutcome::parse(webhooks_payload(deployment_id).to_string().as_bytes()).unwrap();
    service(control_plane, queued(vec![outcome]))
        .sync_all_deployments()
        .await
        .expect("a failed relay does not fail the cycle");

    let logs = captured.text();
    assert!(logs.contains("503"), "{logs}");
    assert!(!logs.contains(SECRET), "{logs}");
}

#[tokio::test]
async fn an_smtp_theme_or_iam_outcome_is_forwarded_verbatim_and_never_logged() {
    for kind in ["smtp", "portal_theme", "iam"] {
        let captured = Captured::default();
        let _logs = capture_logs(&captured);
        let deployment_id = Uuid::new_v4();
        let payload = json!({
            "deployment_id": deployment_id,
            "kind": kind,
            "phase": "Done",
            "envelope": { "ct": SECRET },
        });
        let forwarded: Arc<StdMutex<Vec<Value>>> = Arc::default();

        let mut control_plane = control_plane(Vec::new());
        control_plane.expect_report_outcome().never();
        control_plane.expect_report_webhooks_outcome().never();
        let seen = Arc::clone(&forwarded);
        control_plane
            .expect_report_relayed_outcome()
            .times(1)
            .returning(move |_, outcome| {
                assert_eq!(outcome.deployment_id(), deployment_id);
                seen.lock().unwrap().push(outcome.body().clone());
                Box::pin(async { Ok(()) })
            });

        let outcome = InboundOutcome::parse(payload.to_string().as_bytes()).unwrap();
        service(control_plane, queued(vec![outcome]))
            .sync_all_deployments()
            .await
            .expect("cycle succeeds");

        assert_eq!(*forwarded.lock().unwrap(), vec![payload], "{kind}");
        let logs = captured.text();
        assert!(logs.contains(kind), "{logs}");
        assert!(!logs.contains(SECRET), "{logs}");
    }
}

#[tokio::test]
async fn a_legacy_report_still_goes_to_the_lifecycle_endpoint() {
    let deployment_id = Uuid::new_v4();

    let mut control_plane = control_plane(Vec::new());
    control_plane.expect_report_webhooks_outcome().never();
    control_plane
        .expect_report_outcome()
        .times(1)
        .withf(move |_, report| {
            report.deployment_id == deployment_id && report.outcome == "running"
        })
        .returning(|_, _| Box::pin(async { Ok(()) }));

    let legacy = DeploymentOutcomeReport::observed(deployment_id, "running", None).into();
    service(control_plane, queued(vec![legacy]))
        .sync_all_deployments()
        .await
        .expect("cycle succeeds");
}

fn deliveries_json(status: &str, updated_at: &str) -> String {
    json!([
        {
            "webhook_id": "w1", "delivery_id": "d1", "event": "user.created",
            "status": status, "attempt_count": 2, "last_status_code": 500,
            "last_error": "boom",
            "created_at": "2026-10-22T08:00:00Z", "updated_at": updated_at
        }
    ])
    .to_string()
}

fn empty_inbox() -> MockOutcomeInboxRepository {
    queued(Vec::new())
}

fn source_returning(
    answers: Vec<Result<Option<String>, HeraldError>>,
) -> (Arc<dyn WebhookDeliverySource>, Asked) {
    let asked: Asked = Arc::default();
    let log = Arc::clone(&asked);
    let answers = StdMutex::new(answers.into_iter());
    let mut source = MockWebhookDeliverySource::new();
    source
        .expect_read()
        .returning(move |namespace, config_map| {
            log.lock().unwrap().push((namespace, config_map));
            let answer = answers
                .lock()
                .unwrap()
                .next()
                .expect("one answer per cycle");
            Box::pin(async move { answer })
        });
    (Arc::new(source), asked)
}

fn recording_sink() -> (Arc<dyn LogIndexSink>, Shipped) {
    let shipped: Shipped = Arc::default();
    let log = Arc::clone(&shipped);
    let mut sink = MockLogIndexSink::new();
    sink.expect_ship()
        .returning(move |organisation, documents: Vec<LogIndexDocument>| {
            let documents = documents
                .iter()
                .map(|d| serde_json::to_value(d).unwrap())
                .collect();
            log.lock().unwrap().push((organisation, documents));
            Box::pin(async { Ok(()) })
        });
    (Arc::new(sink), shipped)
}

fn shipping_service(
    deployments: Vec<Deployment>,
    source: Arc<dyn WebhookDeliverySource>,
    sink: Arc<dyn LogIndexSink>,
) -> Service {
    service(control_plane(deployments), empty_inbox())
        .with_log_index(Some(sink))
        .with_webhook_deliveries(Some(source))
}

fn ferriskey() -> Deployment {
    deployment(Some(DeploymentKind::Ferriskey), Some("autharie-acme"))
}

#[tokio::test]
async fn new_deliveries_are_shipped_from_the_instances_configmap() {
    let (source, asked) = source_returning(vec![Ok(Some(deliveries_json(
        "failed",
        "2026-10-22T08:05:00Z",
    )))]);
    let (sink, shipped) = recording_sink();

    shipping_service(vec![ferriskey()], source, sink)
        .sync_all_deployments()
        .await
        .unwrap();

    assert_eq!(
        *asked.lock().unwrap(),
        vec![(
            "autharie-acme".to_string(),
            "deployment-11111111-1111-1111-1111-111111111111-webhook-deliveries".to_string()
        )]
    );
    let shipped = shipped.lock().unwrap();
    assert_eq!(shipped.len(), 1);
    assert_eq!(
        shipped[0].0,
        OrganisationId::new("dddddddd-dddd-dddd-dddd-dddddddddddd")
    );
    let document = &shipped[0].1[0];
    assert_eq!(document["kind"], "webhook_delivery");
    assert_eq!(
        document["deployment_id"],
        "11111111-1111-1111-1111-111111111111"
    );
    assert_eq!(document["delivery_id"], "d1");
    assert_eq!(document["timestamp"], "2026-10-22T08:05:00Z");
}

#[tokio::test]
async fn an_unchanged_delivery_is_shipped_once_and_a_changed_one_again() {
    let (source, _) = source_returning(vec![
        Ok(Some(deliveries_json("delivering", "2026-10-22T08:00:00Z"))),
        Ok(Some(deliveries_json("delivering", "2026-10-22T08:00:00Z"))),
        Ok(Some(deliveries_json("failed", "2026-10-22T08:05:00Z"))),
    ]);
    let (sink, shipped) = recording_sink();
    let service = shipping_service(vec![ferriskey()], source, sink);

    for _ in 0..3 {
        service.sync_all_deployments().await.unwrap();
    }

    let shipped = shipped.lock().unwrap();
    assert_eq!(shipped.len(), 2);
    assert_eq!(shipped[0].1[0]["status"], "delivering");
    assert_eq!(shipped[1].1[0]["status"], "failed");
}

#[tokio::test]
async fn a_restarted_herald_ships_everything_again() {
    let answer = || Ok(Some(deliveries_json("failed", "2026-10-22T08:05:00Z")));
    let (sink, shipped) = recording_sink();

    for _ in 0..2 {
        let (source, _) = source_returning(vec![answer()]);
        shipping_service(vec![ferriskey()], source, Arc::clone(&sink))
            .sync_all_deployments()
            .await
            .unwrap();
    }

    assert_eq!(shipped.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_missing_configmap_is_no_webhooks_not_an_error() {
    let (source, _) = source_returning(vec![Ok(None)]);
    let (sink, shipped) = recording_sink();

    shipping_service(vec![ferriskey()], source, sink)
        .sync_all_deployments()
        .await
        .expect("cycle succeeds");

    assert!(shipped.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_malformed_configmap_is_skipped_with_a_warning() {
    let captured = Captured::default();
    let _logs = capture_logs(&captured);
    let (source, _) = source_returning(vec![Ok(Some("{not json".to_string()))]);
    let (sink, shipped) = recording_sink();

    shipping_service(vec![ferriskey()], source, sink)
        .sync_all_deployments()
        .await
        .expect("cycle succeeds");

    assert!(shipped.lock().unwrap().is_empty());
    assert!(captured.text().contains("malformed"), "{}", captured.text());
}

#[tokio::test]
async fn an_unreadable_configmap_does_not_fail_the_cycle() {
    let (source, _) = source_returning(vec![Err(HeraldError::Internal {
        message: "forbidden".to_string(),
    })]);
    let (sink, shipped) = recording_sink();

    shipping_service(vec![ferriskey()], source, sink)
        .sync_all_deployments()
        .await
        .expect("cycle succeeds");

    assert!(shipped.lock().unwrap().is_empty());
}

#[tokio::test]
async fn keycloak_and_unplaced_deployments_are_not_read() {
    let source = MockWebhookDeliverySource::new();
    let (sink, shipped) = recording_sink();

    shipping_service(
        vec![
            deployment(Some(DeploymentKind::Keycloak), Some("autharie-acme")),
            deployment(Some(DeploymentKind::Ferriskey), None),
        ],
        Arc::new(source),
        sink,
    )
    .sync_all_deployments()
    .await
    .unwrap();

    assert!(shipped.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_shipment_is_retried_next_cycle() {
    let answer = || Ok(Some(deliveries_json("failed", "2026-10-22T08:05:00Z")));
    let (source, _) = source_returning(vec![answer(), answer()]);

    let attempts = Arc::new(StdMutex::new(0));
    let counted = Arc::clone(&attempts);
    let mut sink = MockLogIndexSink::new();
    sink.expect_ship().returning(move |_, _| {
        let mut attempts = counted.lock().unwrap();
        *attempts += 1;
        let first = *attempts == 1;
        Box::pin(async move {
            if first {
                Err(HeraldError::Internal {
                    message: "quickwit down".to_string(),
                })
            } else {
                Ok(())
            }
        })
    });

    let service = shipping_service(vec![ferriskey()], source, Arc::new(sink));
    service.sync_all_deployments().await.unwrap();
    service.sync_all_deployments().await.unwrap();

    assert_eq!(*attempts.lock().unwrap(), 2);
}
