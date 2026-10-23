use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::*;
use crate::{
    CoreError,
    action::{Action, ports::MockActionRepository},
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
    organisation::OrganisationId,
    smtp::{
        ports::{SmtpOutcomeService, SmtpPolicy, SmtpRepository, SmtpService},
        service::{SmtpOutcomeServiceImpl, SmtpServiceImpl},
        value_objects::parse_realm,
    },
    user::UserId,
    version::Version,
    webhooks::ports::{KnownRealms, SecretVault},
};

const ORGANISATION: Uuid = Uuid::from_u128(1);
const DEPLOYMENT: Uuid = Uuid::from_u128(2);
const DATAPLANE: Uuid = Uuid::from_u128(3);
const PASSWORD: &str = "hunter2-canary";

#[derive(Default)]
struct State {
    revisions: HashMap<String, u64>,
    configs: HashMap<String, StoredSmtp>,
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<State>>);

impl Memory {
    fn state<T>(&self, read: impl FnOnce(&State) -> T) -> T {
        read(&self.0.lock().unwrap())
    }

    fn stored(&self, realm: &str) -> Option<StoredSmtp> {
        self.state(|state| state.configs.get(realm).cloned())
    }

    fn revision_of(&self, realm: &str) -> u64 {
        self.state(|state| state.revisions.get(realm).copied().unwrap_or_default())
    }
}

impl SmtpRepository for Memory {
    async fn next_revision(&self, _: DeploymentId, realm: &SmtpRealm) -> Result<u64, CoreError> {
        let mut state = self.0.lock().unwrap();
        let revision = state
            .revisions
            .entry(realm.as_str().to_string())
            .or_default();
        *revision += 1;
        Ok(*revision)
    }

    async fn revision(&self, _: DeploymentId, realm: &SmtpRealm) -> Result<u64, CoreError> {
        Ok(self.revision_of(realm.as_str()))
    }

    async fn get(
        &self,
        _: DeploymentId,
        realm: &SmtpRealm,
    ) -> Result<Option<StoredSmtp>, CoreError> {
        Ok(self.stored(realm.as_str()))
    }

    async fn upsert(&self, stored: &StoredSmtp) -> Result<(), CoreError> {
        self.0
            .lock()
            .unwrap()
            .configs
            .insert(stored.config.realm.as_str().to_string(), stored.clone());
        Ok(())
    }

    async fn delete(&self, _: DeploymentId, realm: &SmtpRealm) -> Result<bool, CoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .configs
            .remove(realm.as_str())
            .is_some())
    }

    async fn set_state(
        &self,
        _: DeploymentId,
        realm: &SmtpRealm,
        state: &SmtpState,
        _: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut held = self.0.lock().unwrap();
        match held.configs.get_mut(realm.as_str()) {
            Some(stored) => {
                stored.config.state = state.clone();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

struct Realms(Vec<&'static str>);

impl KnownRealms for Realms {
    async fn known_realms(&self, _: DeploymentId) -> Result<Vec<String>, CoreError> {
        Ok(self.0.iter().map(|realm| realm.to_string()).collect())
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

impl SmtpPolicy for Policy {
    async fn can_view_smtp(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        Ok(())
    }

    async fn can_manage_smtp(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        if self.manage {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied {
                reason: "insufficient permissions".to_string(),
            })
        }
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
}

impl Written {
    fn smtp(&self) -> Vec<Action> {
        self.actions
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.action_type.0 == SMTP_ACTION_TYPE)
            .cloned()
            .collect()
    }
}

type Service =
    SmtpServiceImpl<MockDeploymentRepository, Memory, Realms, MockActionRepository, Vault, Policy>;

struct World {
    service: Service,
    memory: Memory,
    written: Written,
}

fn world(kind: DeploymentKind, manage: bool) -> World {
    let memory = Memory::default();
    let written = Written::default();

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

    World {
        service: SmtpServiceImpl::new(
            deployments,
            memory.clone(),
            Realms(vec!["master", "customers"]),
            actions,
            Vault,
            Policy { manage },
        ),
        memory,
        written,
    }
}

fn ferriskey() -> World {
    world(DeploymentKind::Ferriskey, true)
}

fn raw(password: Option<&str>) -> RawSmtpInput {
    RawSmtpInput {
        host: "smtp.example.com".to_string(),
        port: 587,
        encryption: "starttls".to_string(),
        username: "apikey".to_string(),
        password: password.map(str::to_string),
        from_email: "no-reply@example.com".to_string(),
        from_name: "Acme".to_string(),
    }
}

fn input(realm: &str, password: Option<&str>) -> SmtpInput {
    SmtpInput::parse(realm, raw(password)).unwrap()
}

async fn save(world: &World, realm: &str, password: Option<&str>) -> Result<SmtpView, CoreError> {
    world
        .service
        .save_smtp(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            input(realm, password),
        )
        .await
}

async fn remove(world: &World, realm: &str) -> Result<(), CoreError> {
    world
        .service
        .remove_smtp(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            parse_realm(realm).unwrap(),
        )
        .await
}

async fn read(world: &World, realm: &str) -> Option<SmtpView> {
    world
        .service
        .get_smtp(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            parse_realm(realm).unwrap(),
        )
        .await
        .expect("read")
}

fn ingest_as(
    world: &World,
    speaking: Uuid,
) -> SmtpOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory> {
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

    SmtpOutcomeServiceImpl::new(deployments, dataplanes, world.memory.clone())
}

fn outcome(revision: u64, realm: &str, phase: &str, message: Option<&str>) -> SmtpOutcome {
    SmtpOutcome::parse(
        revision,
        realm,
        vec![(phase.to_string(), message.map(str::to_string))],
    )
    .unwrap()
}

async fn report(world: &World, outcome: SmtpOutcome) -> bool {
    ingest_as(world, DATAPLANE)
        .report_smtp_outcome(caller(), DeploymentId(DEPLOYMENT), outcome)
        .await
        .expect("reported")
}

fn state_of(world: &World, realm: &str) -> SmtpState {
    world.memory.stored(realm).unwrap().config.state
}

fn is_unsupported(error: &CoreError) -> bool {
    matches!(error, CoreError::Smtp(SmtpError::UnsupportedKind))
}

#[tokio::test]
async fn the_first_save_needs_a_password() {
    let world = ferriskey();

    let error = save(&world, "master", None).await.unwrap_err();

    assert!(matches!(
        error,
        CoreError::Smtp(SmtpError::PasswordRequired)
    ));
    assert!(world.written.smtp().is_empty());
    assert!(world.memory.stored("master").is_none());
}

#[tokio::test]
async fn a_save_is_pending_and_sends_the_contract_payload_with_the_password() {
    let world = ferriskey();

    let view = save(&world, "master", Some(PASSWORD)).await.unwrap();

    assert_eq!(view.state, SmtpState::Pending);
    assert!(view.password_set);
    let sent = world.written.smtp();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].payload.data,
        serde_json::json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "production-auth",
            "revision": 1,
            "realm": "master",
            "smtp": {
                "host": "smtp.example.com",
                "port": 587,
                "encryption": "starttls",
                "username": "apikey",
                "from_email": "no-reply@example.com",
                "from_name": "Acme",
                "password": PASSWORD
            }
        })
    );
}

#[tokio::test]
async fn the_password_is_stored_sealed_under_its_own_context_and_never_in_the_view() {
    let world = ferriskey();

    let view = save(&world, "master", Some(PASSWORD)).await.unwrap();

    let sealed = world.memory.stored("master").unwrap().sealed_password;
    assert!(!String::from_utf8_lossy(&sealed.ciphertext).contains(PASSWORD));
    assert_eq!(
        sealed.wrapped_dek.as_str(),
        format!("smtp-password:{DEPLOYMENT}:master")
    );
    assert!(!serde_json::to_string(&view).unwrap().contains(PASSWORD));
    assert!(!format!("{view:?}").contains(PASSWORD));
}

#[tokio::test]
async fn saving_again_without_a_password_keeps_the_sealed_one_and_sends_null() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    let before = world.memory.stored("master").unwrap().sealed_password;

    let mut change = raw(None);
    change.from_name = "Acme Support".to_string();
    world
        .service
        .save_smtp(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            SmtpInput::parse("master", change).unwrap(),
        )
        .await
        .unwrap();

    let after = world.memory.stored("master").unwrap();
    assert_eq!(after.sealed_password, before);
    assert_eq!(after.config.settings.from_name.as_str(), "Acme Support");
    let sent = world.written.smtp();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].payload.data["revision"], 2);
    assert!(sent[1].payload.data["smtp"]["password"].is_null());
    assert!(!sent[1].payload.data.to_string().contains(PASSWORD));
}

#[tokio::test]
async fn saving_with_a_new_password_replaces_the_sealed_one() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    let before = world.memory.stored("master").unwrap().sealed_password;

    save(&world, "master", Some("another-password"))
        .await
        .unwrap();

    assert_ne!(
        world.memory.stored("master").unwrap().sealed_password,
        before
    );
    assert_eq!(
        world.written.smtp()[1].payload.data["smtp"]["password"],
        "another-password"
    );
}

#[tokio::test]
async fn each_realm_has_its_own_config_and_revision() {
    let world = ferriskey();

    save(&world, "master", Some(PASSWORD)).await.unwrap();
    save(&world, "customers", Some(PASSWORD)).await.unwrap();
    save(&world, "customers", None).await.unwrap();

    assert_eq!(world.memory.revision_of("master"), 1);
    assert_eq!(world.memory.revision_of("customers"), 2);
    assert!(read(&world, "master").await.is_some());
}

#[tokio::test]
async fn a_realm_the_deployment_does_not_have_is_refused() {
    let world = ferriskey();

    let error = save(&world, "elsewhere", Some(PASSWORD)).await.unwrap_err();

    assert!(matches!(error, CoreError::Smtp(SmtpError::Invalid { .. })));
    assert!(world.written.smtp().is_empty());
}

#[tokio::test]
async fn removing_sends_a_null_smtp_and_forgets_the_config() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    remove(&world, "master").await.unwrap();

    assert!(read(&world, "master").await.is_none());
    let sent = world.written.smtp();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].payload.data["revision"], 2);
    assert!(sent[1].payload.data["smtp"].is_null());
    assert_eq!(sent[1].payload.data["realm"], "master");
}

#[tokio::test]
async fn removing_what_is_not_there_is_not_found() {
    let world = ferriskey();

    let error = remove(&world, "master").await.unwrap_err();

    assert!(matches!(error, CoreError::Smtp(SmtpError::NotFound { .. })));
    assert!(world.written.smtp().is_empty());
}

#[tokio::test]
async fn the_revision_keeps_growing_across_a_removal_and_a_new_save() {
    let world = ferriskey();

    save(&world, "master", Some(PASSWORD)).await.unwrap();
    remove(&world, "master").await.unwrap();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    let revisions: Vec<_> = world
        .written
        .smtp()
        .iter()
        .map(|action| action.payload.data["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2, 3]);
}

#[tokio::test]
async fn a_recreated_config_needs_a_password_again() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    remove(&world, "master").await.unwrap();

    let error = save(&world, "master", None).await.unwrap_err();

    assert!(matches!(
        error,
        CoreError::Smtp(SmtpError::PasswordRequired)
    ));
}

#[tokio::test]
async fn a_viewer_reads_but_cannot_change() {
    let world = world(DeploymentKind::Ferriskey, false);

    assert!(read(&world, "master").await.is_none());
    assert!(matches!(
        save(&world, "master", Some(PASSWORD)).await.unwrap_err(),
        CoreError::PermissionDenied { .. }
    ));
    assert!(matches!(
        remove(&world, "master").await.unwrap_err(),
        CoreError::PermissionDenied { .. }
    ));
    assert!(world.written.smtp().is_empty());
}

#[tokio::test]
async fn another_kind_of_deployment_is_refused() {
    let world = world(DeploymentKind::Keycloak, true);

    assert!(is_unsupported(
        &save(&world, "master", Some(PASSWORD)).await.unwrap_err()
    ));
    assert!(is_unsupported(&remove(&world, "master").await.unwrap_err()));
    assert!(read(&world, "master").await.is_none());
    assert!(world.written.smtp().is_empty());
}

#[tokio::test]
async fn the_realms_are_those_the_deployment_has() {
    let world = ferriskey();

    let realms = world
        .service
        .branding_realms(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
        )
        .await
        .unwrap();

    assert_eq!(realms, vec!["master".to_string(), "customers".to_string()]);
}

#[tokio::test]
async fn a_newer_outcome_sets_applied_or_failed() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    assert!(report(&world, outcome(1, "master", "Applied", None)).await);
    assert_eq!(state_of(&world, "master"), SmtpState::Applied);

    save(&world, "master", None).await.unwrap();
    assert!(report(&world, outcome(2, "master", "Failed", Some("refused"))).await);
    assert_eq!(
        state_of(&world, "master"),
        SmtpState::Failed {
            message: "refused".to_string()
        }
    );
}

#[tokio::test]
async fn an_older_outcome_changes_nothing() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    save(&world, "master", None).await.unwrap();

    assert!(!report(&world, outcome(1, "master", "Applied", None)).await);

    assert_eq!(state_of(&world, "master"), SmtpState::Pending);
}

#[tokio::test]
async fn an_outcome_for_the_same_state_records_nothing() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    assert!(!report(&world, outcome(1, "master", "Pending", None)).await);
}

#[tokio::test]
async fn a_removal_outcome_for_a_deleted_config_is_a_no_op() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    remove(&world, "master").await.unwrap();

    assert!(!report(&world, outcome(2, "master", "Applied", None)).await);

    assert!(world.memory.stored("master").is_none());
}

#[tokio::test]
async fn an_outcome_of_a_removed_config_does_not_touch_its_successor() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    remove(&world, "master").await.unwrap();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    assert!(!report(&world, outcome(2, "master", "Applied", None)).await);

    assert_eq!(state_of(&world, "master"), SmtpState::Pending);
}

#[tokio::test]
async fn an_outcome_only_touches_its_own_realm() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();
    save(&world, "customers", Some(PASSWORD)).await.unwrap();

    assert!(report(&world, outcome(1, "customers", "Applied", None)).await);

    assert_eq!(state_of(&world, "master"), SmtpState::Pending);
    assert_eq!(state_of(&world, "customers"), SmtpState::Applied);
}

#[tokio::test]
async fn an_outcome_from_another_data_plane_is_refused() {
    let world = ferriskey();
    save(&world, "master", Some(PASSWORD)).await.unwrap();

    let error = ingest_as(&world, Uuid::from_u128(99))
        .report_smtp_outcome(
            caller(),
            DeploymentId(DEPLOYMENT),
            outcome(1, "master", "Applied", None),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, CoreError::PermissionDenied { .. }));
    assert_eq!(state_of(&world, "master"), SmtpState::Pending);
}
