use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::{
    CoreError,
    action::{Action, ports::MockActionRepository},
    audit::{AuditEntry, ports::MockAuditRepository},
    backups::keys::{KeyName, KeyRef, KeyVersion, ProviderName, WrappedDek},
    dataplane::{
        credential::SecretString,
        credential_repository::SealedSecret,
        entities::DataPlane,
        ports::MockDataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, DeploymentResources, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::MockDeploymentRepository,
    },
    organisation::{
        Organisation, OrganisationId,
        ports::MockOrganisationRepository,
        value_objects::{OrganisationName, OrganisationSlug, Plan},
    },
    user::{User, UserId, ports::UserRepository},
    version::Version,
    webhooks::{
        commands::{
            RawWebhookInput, RequestReport, SecretReport, WebhookInput, WebhookReport,
            WebhooksOutcome,
        },
        delivery::WebhookDeliveryIndex,
        ports::{
            SecretVault, WebhookOutcomeService, WebhookPolicy, WebhookRepository, WebhookService,
        },
        service::{WebhookOutcomeServiceImpl, WebhookServiceImpl},
    },
};

const ORGANISATION: Uuid = Uuid::from_u128(1);
const DEPLOYMENT: Uuid = Uuid::from_u128(2);
const DATAPLANE: Uuid = Uuid::from_u128(3);
const HEADER_VALUE: &str = "hunter2-header";
const SECRET: &str = "whsec_hunter2";

#[derive(Default)]
struct State {
    revision: u64,
    applied: u64,
    realms: Option<Vec<String>>,
    webhooks: Vec<StoredWebhook>,
    secrets: HashMap<WebhookId, SecretRecord>,
    requests: Vec<WebhookRequest>,
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<State>>);

impl Memory {
    fn with_secret(&self, mut stored: StoredWebhook) -> StoredWebhook {
        let state = self.0.lock().unwrap();
        let record = state.secrets.get(&stored.webhook.id);
        stored.webhook.secret_version = record.map(|record| record.version);
        stored.webhook.secret_received_at = record.map(|record| record.received_at);
        stored
    }

    fn state<T>(&self, read: impl FnOnce(&State) -> T) -> T {
        read(&self.0.lock().unwrap())
    }
}

impl WebhookRepository for Memory {
    async fn next_revision(&self, _: DeploymentId) -> Result<u64, CoreError> {
        let mut state = self.0.lock().unwrap();
        state.revision += 1;
        Ok(state.revision)
    }

    async fn sync_state(&self, _: DeploymentId) -> Result<WebhookSync, CoreError> {
        let state = self.0.lock().unwrap();
        Ok(WebhookSync {
            revision: state.revision,
            applied_revision: state.applied,
            realms: state.realms.clone(),
        })
    }

    async fn record_applied(
        &self,
        _: DeploymentId,
        revision: u64,
        realms: &[String],
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mut state = self.0.lock().unwrap();
        state.applied = state.applied.max(revision);
        state.realms = (!realms.is_empty()).then(|| realms.to_vec());
        Ok(())
    }

    async fn list(&self, _: DeploymentId) -> Result<Vec<StoredWebhook>, CoreError> {
        let all = self.state(|state| state.webhooks.clone());
        Ok(all
            .into_iter()
            .map(|stored| self.with_secret(stored))
            .collect())
    }

    async fn get(
        &self,
        _: DeploymentId,
        id: WebhookId,
    ) -> Result<Option<StoredWebhook>, CoreError> {
        let found = self.state(|state| {
            state
                .webhooks
                .iter()
                .find(|stored| stored.webhook.id == id)
                .cloned()
        });
        Ok(found.map(|stored| self.with_secret(stored)))
    }

    async fn insert(&self, webhook: &StoredWebhook) -> Result<(), CoreError> {
        self.0.lock().unwrap().webhooks.push(webhook.clone());
        Ok(())
    }

    async fn update(&self, webhook: &StoredWebhook) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state
            .webhooks
            .iter_mut()
            .find(|stored| stored.webhook.id == webhook.webhook.id)
        {
            Some(slot) => {
                *slot = webhook.clone();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete(&self, _: DeploymentId, id: WebhookId) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        let before = state.webhooks.len();
        state.webhooks.retain(|stored| stored.webhook.id != id);
        state.secrets.remove(&id);
        Ok(state.webhooks.len() != before)
    }

    async fn set_state(
        &self,
        id: WebhookId,
        new_state: &WebhookState,
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mut state = self.0.lock().unwrap();
        if let Some(stored) = state.webhooks.iter_mut().find(|s| s.webhook.id == id) {
            stored.webhook.state = new_state.clone();
        }
        Ok(())
    }

    async fn secret(&self, id: WebhookId) -> Result<Option<SecretRecord>, CoreError> {
        Ok(self.state(|state| state.secrets.get(&id).cloned()))
    }

    async fn store_secret(&self, id: WebhookId, secret: &SecretRecord) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        if state
            .secrets
            .get(&id)
            .is_some_and(|stored| stored.version >= secret.version)
        {
            return Ok(false);
        }
        state.secrets.insert(id, secret.clone());
        Ok(true)
    }

    async fn insert_request(&self, request: &WebhookRequest) -> Result<(), CoreError> {
        self.0.lock().unwrap().requests.push(request.clone());
        Ok(())
    }

    async fn finish_request(
        &self,
        id: Uuid,
        phase: RequestPhase,
        message: Option<&str>,
        _: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut state = self.0.lock().unwrap();
        match state.requests.iter_mut().find(|request| request.id == id) {
            Some(request) => {
                request.phase = phase;
                request.message = message.map(str::to_string);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn recent_requests(
        &self,
        _: DeploymentId,
        _: usize,
    ) -> Result<Vec<WebhookRequest>, CoreError> {
        Ok(self.state(|state| state.requests.clone()))
    }
}

struct Vault;

fn key() -> KeyRef {
    KeyRef::new(
        ProviderName::platform(),
        KeyName::new("test").unwrap(),
        KeyVersion::new(1),
    )
}

impl SecretVault for Vault {
    async fn seal(
        &self,
        context: &str,
        plaintext: &SecretString,
    ) -> Result<SealedSecret, CoreError> {
        let mut bytes = plaintext.expose().as_bytes().to_vec();
        bytes.reverse();
        Ok(SealedSecret {
            ciphertext: bytes,
            nonce: vec![0; 12],
            wrapped_dek: WrappedDek::new(context),
            key: key(),
        })
    }

    async fn open(&self, context: &str, sealed: &SealedSecret) -> Result<SecretString, CoreError> {
        if sealed.wrapped_dek.as_str() != context {
            return Err(CoreError::InternalError("wrong context".to_string()));
        }
        let mut bytes = sealed.ciphertext.clone();
        bytes.reverse();
        Ok(SecretString::new(String::from_utf8(bytes).unwrap()))
    }
}

#[derive(Clone, Copy)]
struct Policy {
    manage: bool,
}

impl WebhookPolicy for Policy {
    async fn can_view_webhooks(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        Ok(())
    }

    async fn can_manage_webhooks(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        if self.manage {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied {
                reason: "insufficient permissions".to_string(),
            })
        }
    }
}

struct Users;

impl UserRepository for Users {
    async fn upsert_by_email(&self, user: &User) -> Result<User, CoreError> {
        Ok(user.clone())
    }

    async fn find_by_sub(&self, sub: &str) -> Result<Option<User>, CoreError> {
        Ok(Some(User {
            id: UserId(Uuid::from_u128(9)),
            email: "a@b.c".to_string(),
            name: "a".to_string(),
            sub: sub.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn find_by_email(&self, _: &str) -> Result<Option<User>, CoreError> {
        Ok(None)
    }
}

fn caller() -> Identity {
    Identity::User(autharie_auth::User {
        id: "user".to_string(),
        username: "user".to_string(),
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

#[derive(Clone, Default)]
struct Written {
    actions: Arc<Mutex<Vec<Action>>>,
    audit: Arc<Mutex<Vec<AuditEntry>>>,
}

impl Written {
    fn actions(&self) -> Vec<Action> {
        self.actions.lock().unwrap().clone()
    }

    fn of_type(&self, action_type: &str) -> Vec<Action> {
        self.actions()
            .into_iter()
            .filter(|action| action.action_type.0 == action_type)
            .collect()
    }

    fn audited(&self) -> Vec<String> {
        self.audit
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.action.0.clone())
            .collect()
    }
}

type Service = WebhookServiceImpl<
    MockDeploymentRepository,
    MockOrganisationRepository,
    Memory,
    MockActionRepository,
    MockAuditRepository,
    Users,
    Vault,
    Policy,
>;

struct World {
    service: Service,
    memory: Memory,
    written: Written,
}

fn world(kind: DeploymentKind, plan: Plan, manage: bool) -> World {
    let memory = Memory::default();
    let written = Written::default();

    let mut deployments = MockDeploymentRepository::new();
    deployments.expect_get_by_id().returning(move |_| {
        let found = deployment(kind.clone());
        Box::pin(async move { Ok(Some(found)) })
    });

    let mut organisations = MockOrganisationRepository::new();
    organisations.expect_find_by_id().returning(move |_| {
        let mut organisation = Organisation::new(
            OrganisationName::new("acme").unwrap(),
            OrganisationSlug::new("acme").unwrap(),
            UserId(Uuid::from_u128(4)),
            plan,
        );
        organisation.id = OrganisationId(ORGANISATION);
        Box::pin(async move { Ok(Some(organisation)) })
    });

    let mut actions = MockActionRepository::new();
    let held = Arc::clone(&written.actions);
    actions.expect_append().returning(move |action| {
        held.lock().unwrap().push(action);
        Box::pin(async { Ok(()) })
    });

    let mut audit = MockAuditRepository::new();
    let held = Arc::clone(&written.audit);
    audit.expect_append().returning(move |entry| {
        held.lock().unwrap().push(entry);
        Box::pin(async { Ok(()) })
    });

    World {
        service: WebhookServiceImpl::new(
            deployments,
            organisations,
            memory.clone(),
            actions,
            audit,
            Users,
            Vault,
            Policy { manage },
        ),
        memory,
        written,
    }
}

fn ferriskey() -> World {
    world(DeploymentKind::Ferriskey, Plan::Starter, true)
}

fn raw(name: &str) -> RawWebhookInput {
    RawWebhookInput {
        realm: None,
        name: name.to_string(),
        description: None,
        endpoint: "https://example.com/hook".to_string(),
        events: vec!["user.created".to_string()],
        headers: vec![],
    }
}

fn input(name: &str) -> WebhookInput {
    WebhookInput::parse(raw(name)).unwrap()
}

fn input_with_header(name: &str, value: Option<&str>) -> WebhookInput {
    let mut raw = raw(name);
    raw.headers = vec![("x-api-key".to_string(), value.map(str::to_string))];
    WebhookInput::parse(raw).unwrap()
}

async fn create(world: &World, input: WebhookInput) -> WebhookView {
    world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input,
        )
        .await
        .expect("created")
}

fn outcome(revision: u64, webhooks: Vec<WebhookReport>) -> WebhooksOutcome {
    WebhooksOutcome {
        revision,
        webhooks,
        secrets: vec![],
        realms: vec!["master".to_string()],
        requests: vec![],
    }
}

fn active(id: WebhookId, ferriskey_id: &str) -> WebhookReport {
    WebhookReport {
        id,
        state: WebhookState::Active {
            ferriskey_id: ferriskey_id.to_string(),
        },
    }
}

fn secret_of(id: WebhookId, version: u64, secret: &str) -> SecretReport {
    SecretReport {
        id,
        version,
        secret: SecretString::new(secret),
    }
}

fn ingest(
    world: &World,
) -> WebhookOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory, Vault> {
    ingest_as(world, DATAPLANE)
}

fn ingest_as(
    world: &World,
    speaking: Uuid,
) -> WebhookOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory, Vault> {
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

    WebhookOutcomeServiceImpl::new(deployments, dataplanes, world.memory.clone(), Vault)
}

async fn report(world: &World, outcome: WebhooksOutcome) -> bool {
    ingest(world)
        .report_webhooks_outcome(caller(), DeploymentId(DEPLOYMENT), outcome)
        .await
        .expect("reported")
}

fn webhook_of(world: &World, id: WebhookId) -> Webhook {
    world.memory.state(|state| {
        state
            .webhooks
            .iter()
            .find(|stored| stored.webhook.id == id)
            .unwrap()
            .webhook
            .clone()
    })
}

#[tokio::test]
async fn a_created_webhook_is_pending_and_the_whole_set_is_sent() {
    let world = ferriskey();

    let view = create(&world, input_with_header("orders", Some(HEADER_VALUE))).await;

    assert_eq!(view.webhook.state, WebhookState::Pending);
    let sent = world.written.of_type(WEBHOOKS_ACTION_TYPE);
    assert_eq!(sent.len(), 1);
    let payload = &sent[0].payload.data;
    assert_eq!(payload["revision"], 1);
    assert_eq!(payload["namespace"], "production-auth");
    assert_eq!(payload["webhooks"][0]["id"], json!(view.webhook.id.0));
    assert_eq!(payload["webhooks"][0]["realm"], "master");
    assert_eq!(payload["webhooks"][0]["header_names"], json!(["x-api-key"]));
    assert_eq!(
        payload["headers"][view.webhook.id.0.to_string()]["x-api-key"],
        HEADER_VALUE
    );
    assert_eq!(world.written.audited(), vec!["deployment.webhook.created"]);
}

#[tokio::test]
async fn header_values_are_stored_sealed_and_never_in_the_view() {
    let world = ferriskey();

    let view = create(&world, input_with_header("orders", Some(HEADER_VALUE))).await;

    let stored = world.memory.state(|state| state.webhooks[0].clone());
    let sealed = stored.sealed_headers.expect("sealed");
    assert!(
        !String::from_utf8_lossy(&sealed.ciphertext).contains(HEADER_VALUE),
        "the ciphertext holds the value in clear"
    );
    let serialised = serde_json::to_string(&view).unwrap();
    assert!(!serialised.contains(HEADER_VALUE));
    assert!(serialised.contains("x-api-key"));
}

#[tokio::test]
async fn the_revision_only_grows() {
    let world = ferriskey();
    let first = create(&world, input("a")).await;
    create(&world, input("b")).await;
    world
        .service
        .update_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            first.webhook.id,
            input("a2"),
        )
        .await
        .unwrap();
    world
        .service
        .delete_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            first.webhook.id,
        )
        .await
        .unwrap();

    let revisions: Vec<u64> = world
        .written
        .of_type(WEBHOOKS_ACTION_TYPE)
        .iter()
        .map(|action| action.payload.data["revision"].as_u64().unwrap())
        .collect();

    assert_eq!(revisions, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn deleting_sends_the_remaining_set_and_audits() {
    let world = ferriskey();
    let first = create(&world, input("a")).await;
    let second = create(&world, input("b")).await;

    world
        .service
        .delete_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            first.webhook.id,
        )
        .await
        .unwrap();

    let last = world.written.of_type(WEBHOOKS_ACTION_TYPE).pop().unwrap();
    let ids: Vec<Value> = last.payload.data["webhooks"].as_array().unwrap().to_vec();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0]["id"], json!(second.webhook.id.0));
    assert!(
        world
            .written
            .audited()
            .contains(&"deployment.webhook.deleted".to_string())
    );
}

#[tokio::test]
async fn deleting_an_unknown_webhook_is_not_found() {
    let world = ferriskey();

    let result = world
        .service
        .delete_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            WebhookId(Uuid::new_v4()),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::NotFound { .. }))
    ));
}

#[tokio::test]
async fn a_free_deployment_cannot_hold_a_sixth_webhook() {
    let world = world(DeploymentKind::Ferriskey, Plan::Free, true);
    for index in 0..5 {
        create(&world, input(&format!("hook-{index}"))).await;
    }

    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input("sixth"),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::OrganisationLimitReached {
            max: 5,
            current: 5,
            ..
        })
    ));
}

#[tokio::test]
async fn a_paid_deployment_holds_twenty_five() {
    let world = world(DeploymentKind::Ferriskey, Plan::Business, true);
    for index in 0..25 {
        create(&world, input(&format!("hook-{index}"))).await;
    }

    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input("twenty-sixth"),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::OrganisationLimitReached { max: 25, .. })
    ));
}

#[tokio::test]
async fn keycloak_has_no_webhooks() {
    let world = world(DeploymentKind::Keycloak, Plan::Starter, true);

    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input("a"),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::UnsupportedKind))
    ));
}

#[tokio::test]
async fn an_unknown_realm_list_accepts_master_only() {
    let world = ferriskey();
    let mut other = raw("a");
    other.realm = Some("customers".to_string());

    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            WebhookInput::parse(other).unwrap(),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::Invalid { .. }))
    ));
    assert_eq!(world.memory.state(|state| state.webhooks.len()), 0);
}

#[tokio::test]
async fn a_reported_realm_is_accepted_and_an_unreported_one_is_not() {
    let world = ferriskey();
    report(&world, outcome(0, vec![])).await;
    world.memory.0.lock().unwrap().realms =
        Some(vec!["master".to_string(), "customers".to_string()]);

    let mut known = raw("a");
    known.realm = Some("customers".to_string());
    create(&world, WebhookInput::parse(known).unwrap()).await;

    let mut unknown = raw("b");
    unknown.realm = Some("ghosts".to_string());
    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            WebhookInput::parse(unknown).unwrap(),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::Invalid { .. }))
    ));
}

#[tokio::test]
async fn a_header_without_a_value_is_refused_on_create() {
    let world = ferriskey();

    let result = world
        .service
        .create_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input_with_header("a", None),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::Invalid { .. }))
    ));
}

#[tokio::test]
async fn an_update_keeps_the_value_of_a_header_sent_without_one() {
    let world = ferriskey();
    let view = create(&world, input_with_header("a", Some(HEADER_VALUE))).await;

    world
        .service
        .update_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
            input_with_header("renamed", None),
        )
        .await
        .unwrap();

    let last = world.written.of_type(WEBHOOKS_ACTION_TYPE).pop().unwrap();
    assert_eq!(last.payload.data["webhooks"][0]["name"], "renamed");
    assert_eq!(
        last.payload.data["headers"][view.webhook.id.0.to_string()]["x-api-key"],
        HEADER_VALUE
    );
    assert_eq!(
        webhook_of(&world, view.webhook.id).state,
        WebhookState::Pending
    );
}

#[tokio::test]
async fn an_update_cannot_invent_a_header_without_a_value() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;

    let result = world
        .service
        .update_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
            input_with_header("a", None),
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::Invalid { .. }))
    ));
}

#[tokio::test]
async fn an_outcome_turns_a_pending_webhook_active() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;

    let changed = report(&world, outcome(1, vec![active(view.webhook.id, "fk-1")])).await;

    assert!(changed);
    assert_eq!(
        webhook_of(&world, view.webhook.id).state,
        WebhookState::Active {
            ferriskey_id: "fk-1".to_string()
        }
    );
}

#[tokio::test]
async fn a_stale_outcome_changes_nothing() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    report(&world, outcome(1, vec![active(view.webhook.id, "fk-1")])).await;
    world
        .service
        .update_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
            input("a2"),
        )
        .await
        .unwrap();
    report(&world, outcome(2, vec![active(view.webhook.id, "fk-1")])).await;

    let changed = report(
        &world,
        outcome(
            1,
            vec![WebhookReport {
                id: view.webhook.id,
                state: WebhookState::Failed {
                    reason: "old news".to_string(),
                },
            }],
        ),
    )
    .await;

    assert!(!changed);
    assert_eq!(
        webhook_of(&world, view.webhook.id).state,
        WebhookState::Active {
            ferriskey_id: "fk-1".to_string()
        }
    );
}

#[tokio::test]
async fn an_outcome_older_than_a_webhooks_own_change_does_not_overwrite_its_pending_state() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    world
        .service
        .update_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
            input("a2"),
        )
        .await
        .unwrap();

    report(&world, outcome(1, vec![active(view.webhook.id, "fk-1")])).await;

    assert_eq!(
        webhook_of(&world, view.webhook.id).state,
        WebhookState::Pending
    );
}

#[tokio::test]
async fn an_outcome_for_a_webhook_that_was_deleted_is_ignored() {
    let world = ferriskey();

    let changed = report(
        &world,
        outcome(1, vec![active(WebhookId(Uuid::new_v4()), "fk")]),
    )
    .await;

    assert!(changed, "the revision itself still advances");
    assert_eq!(world.memory.state(|state| state.webhooks.len()), 0);
}

#[tokio::test]
async fn a_secret_is_stored_sealed_and_an_older_version_is_ignored() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    let id = view.webhook.id;

    let mut first = outcome(1, vec![active(id, "fk-1")]);
    first.secrets = vec![secret_of(id, 1, SECRET)];
    report(&world, first).await;

    let record = world.memory.state(|state| state.secrets[&id].clone());
    assert_eq!(record.version, 1);
    assert!(!String::from_utf8_lossy(&record.sealed.ciphertext).contains(SECRET));

    let mut replay = outcome(1, vec![]);
    replay.secrets = vec![secret_of(id, 1, "whsec_replayed")];
    assert!(!report(&world, replay).await);

    let revealed = world
        .service
        .reveal_webhook_secret(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            id,
        )
        .await
        .unwrap();
    assert_eq!(revealed.secret.expose(), SECRET);
    assert_eq!(revealed.version, 1);
}

#[tokio::test]
async fn revealing_before_a_secret_arrives_is_not_found_and_not_audited() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;

    let result = world
        .service
        .reveal_webhook_secret(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::SecretNotReceived))
    ));
    assert!(
        !world
            .written
            .audited()
            .contains(&"deployment.webhook.secret_revealed".to_string())
    );
}

#[tokio::test]
async fn revealing_a_secret_is_audited_without_the_secret() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    let mut received = outcome(1, vec![active(view.webhook.id, "fk-1")]);
    received.secrets = vec![secret_of(view.webhook.id, 1, SECRET)];
    report(&world, received).await;

    world
        .service
        .reveal_webhook_secret(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
        )
        .await
        .unwrap();

    assert!(
        world
            .written
            .audited()
            .contains(&"deployment.webhook.secret_revealed".to_string())
    );
    let recorded = serde_json::to_string(&*world.written.audit.lock().unwrap()).unwrap();
    assert!(!recorded.contains(SECRET));
}

#[tokio::test]
async fn rotating_asks_the_data_plane_and_a_newer_secret_replaces_the_old() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    let id = view.webhook.id;
    let mut first = outcome(1, vec![active(id, "fk-1")]);
    first.secrets = vec![secret_of(id, 1, SECRET)];
    report(&world, first).await;

    let request = world
        .service
        .rotate_webhook_secret(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            id,
        )
        .await
        .unwrap();

    let sent = world.written.of_type(ROTATE_SECRET_ACTION_TYPE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].payload.data["webhook_id"], json!(id.0));
    assert_eq!(sent[0].payload.data["request_id"], json!(request.id));
    assert_eq!(sent[0].payload.data["namespace"], "production-auth");
    assert!(
        world
            .written
            .audited()
            .contains(&"deployment.webhook.secret_rotated".to_string())
    );

    let mut second = outcome(1, vec![]);
    second.secrets = vec![secret_of(id, 2, "whsec_second")];
    second.requests = vec![RequestReport {
        id: request.id,
        phase: RequestPhase::Done,
        message: None,
    }];
    report(&world, second).await;

    let revealed = world
        .service
        .reveal_webhook_secret(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            id,
        )
        .await
        .unwrap();
    assert_eq!(revealed.secret.expose(), "whsec_second");
    assert_eq!(revealed.version, 2);
    let shown = world
        .service
        .get_webhook(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            id,
        )
        .await
        .unwrap();
    assert_eq!(shown.requests[0].phase, RequestPhase::Done);
    assert_eq!(shown.webhook.secret_version, Some(2));
}

#[tokio::test]
async fn retrying_a_delivery_records_the_action() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;

    let request = world
        .service
        .retry_webhook_delivery(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            view.webhook.id,
            "delivery-1".to_string(),
        )
        .await
        .unwrap();

    let sent = world.written.of_type(RETRY_DELIVERY_ACTION_TYPE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].payload.data["delivery_id"], "delivery-1");
    assert_eq!(sent[0].payload.data["request_id"], json!(request.id));
}

#[tokio::test]
async fn a_viewer_may_list_but_not_change_reveal_rotate_or_retry() {
    let world = world(DeploymentKind::Ferriskey, Plan::Starter, false);
    let org = OrganisationId(ORGANISATION);
    let dep = DeploymentId(DEPLOYMENT);
    let id = WebhookId(Uuid::new_v4());

    assert!(
        world
            .service
            .list_webhooks(caller(), org, dep)
            .await
            .is_ok()
    );
    assert!(matches!(
        world
            .service
            .create_webhook(caller(), org, dep, input("a"))
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(matches!(
        world
            .service
            .update_webhook(caller(), org, dep, id, input("a"))
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(matches!(
        world.service.delete_webhook(caller(), org, dep, id).await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(matches!(
        world
            .service
            .rotate_webhook_secret(caller(), org, dep, id)
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(matches!(
        world
            .service
            .reveal_webhook_secret(caller(), org, dep, id)
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(matches!(
        world
            .service
            .retry_webhook_delivery(caller(), org, dep, id, "d".to_string())
            .await,
        Err(CoreError::PermissionDenied { .. })
    ));
    assert!(world.written.actions().is_empty());
}

struct FakeIndex {
    seen: Arc<Mutex<Vec<DeliveryQuery>>>,
}

impl WebhookDeliveryIndex for FakeIndex {
    async fn deliveries(
        &self,
        _: OrganisationId,
        query: DeliveryQuery,
    ) -> Result<Vec<WebhookDelivery>, CoreError> {
        let webhook_id = query.webhook_id;
        self.seen.lock().unwrap().push(query);
        Ok(vec![WebhookDelivery {
            delivery_id: "d-1".to_string(),
            webhook_id,
            event: "user.created".to_string(),
            status: DeliveryStatus::Failed,
            attempt_count: 3,
            last_status_code: Some(500),
            last_error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }])
    }
}

#[tokio::test]
async fn deliveries_come_from_the_index_for_this_webhook_with_a_capped_limit() {
    let world = ferriskey();
    let view = create(&world, input("a")).await;
    let seen = Arc::new(Mutex::new(Vec::new()));

    let deliveries = world
        .service
        .webhook_deliveries(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            DeliveryQuery {
                deployment_id: DeploymentId(Uuid::new_v4()),
                webhook_id: view.webhook.id,
                status: Some(DeliveryStatus::Failed),
                from: None,
                until: None,
                limit: 10_000,
            },
            FakeIndex {
                seen: Arc::clone(&seen),
            },
        )
        .await
        .unwrap();

    assert_eq!(deliveries.len(), 1);
    let asked = seen.lock().unwrap()[0].clone();
    assert_eq!(asked.deployment_id, DeploymentId(DEPLOYMENT));
    assert_eq!(asked.webhook_id, view.webhook.id);
    assert_eq!(asked.limit, 100);
}

#[tokio::test]
async fn deliveries_of_an_unknown_webhook_are_not_asked_for() {
    let world = ferriskey();
    let seen = Arc::new(Mutex::new(Vec::new()));

    let result = world
        .service
        .webhook_deliveries(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            DeliveryQuery {
                deployment_id: DeploymentId(DEPLOYMENT),
                webhook_id: WebhookId(Uuid::new_v4()),
                status: None,
                from: None,
                until: None,
                limit: 100,
            },
            FakeIndex {
                seen: Arc::clone(&seen),
            },
        )
        .await;

    assert!(matches!(
        result,
        Err(CoreError::Webhook(WebhookError::NotFound { .. }))
    ));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn realms_default_to_master_until_reported() {
    let world = ferriskey();
    let org = OrganisationId(ORGANISATION);
    let dep = DeploymentId(DEPLOYMENT);

    assert_eq!(
        world
            .service
            .webhook_realms(caller(), org, dep)
            .await
            .unwrap(),
        vec!["master".to_string()]
    );

    let mut reported = outcome(0, vec![]);
    reported.realms = vec!["master".to_string(), "customers".to_string()];
    report(&world, reported).await;

    assert_eq!(
        world
            .service
            .webhook_realms(caller(), org, dep)
            .await
            .unwrap(),
        vec!["master".to_string(), "customers".to_string()]
    );
}

#[tokio::test]
async fn an_empty_realm_list_does_not_forget_the_known_one() {
    let world = ferriskey();
    let mut reported = outcome(0, vec![]);
    reported.realms = vec!["master".to_string(), "customers".to_string()];
    report(&world, reported).await;

    let mut empty = outcome(1, vec![]);
    empty.realms = vec![];
    report(&world, empty).await;

    assert_eq!(
        world.memory.state(|state| state.realms.clone()),
        Some(vec!["master".to_string(), "customers".to_string()])
    );
}

#[tokio::test]
async fn another_data_planes_outcome_is_refused() {
    let world = ferriskey();

    let result = ingest_as(&world, Uuid::from_u128(77))
        .report_webhooks_outcome(caller(), DeploymentId(DEPLOYMENT), outcome(1, vec![]))
        .await;

    assert!(matches!(result, Err(CoreError::PermissionDenied { .. })));
}
