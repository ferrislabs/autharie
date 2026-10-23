use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use autharie_auth::Identity;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::{
    CoreError,
    action::{Action, ActionVersion, TargetKind, ports::MockActionRepository},
    billing::{
        Money, Subscription, SubscriptionTerms, TierCode, ports::MockSubscriptionRepository,
    },
    dataplane::{
        entities::DataPlane,
        ports::MockDataPlaneRepository,
        value_objects::{Capacity, DataPlaneAllocation, DataPlaneId, Region},
    },
    deployments::{
        Deployment, DeploymentId, DeploymentKind, DeploymentName, DeploymentStatus,
        ports::MockDeploymentRepository,
    },
    offers::Offer,
    organisation::{OrganisationId, features::IamFeature},
    themes::{
        ports::{ThemeOutcomeService, ThemePolicy, ThemeRepository, ThemeService},
        service::{ThemeOutcomeServiceImpl, ThemeServiceImpl},
    },
    user::UserId,
    version::Version,
    webhooks::ports::KnownRealms,
};

const ORGANISATION: Uuid = Uuid::from_u128(1);
const DEPLOYMENT: Uuid = Uuid::from_u128(2);
const DATAPLANE: Uuid = Uuid::from_u128(3);

#[derive(Default)]
struct State {
    themes: Vec<Theme>,
    syncs: HashMap<String, ThemeSync>,
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<State>>);

impl Memory {
    fn themes_of(&self, realm: &str) -> Vec<Theme> {
        self.0
            .lock()
            .unwrap()
            .themes
            .iter()
            .filter(|theme| theme.realm.as_str() == realm)
            .cloned()
            .collect()
    }

    fn sync_of(&self, realm: &str) -> Option<ThemeSync> {
        self.0.lock().unwrap().syncs.get(realm).cloned()
    }
}

fn clash(held: &State, theme: &Theme) -> bool {
    held.themes.iter().any(|other| {
        other.id != theme.id && other.realm == theme.realm && other.name.key() == theme.name.key()
    })
}

impl ThemeRepository for Memory {
    async fn lock_realm(&self, _: DeploymentId, _: &ThemeRealm) -> Result<(), CoreError> {
        Ok(())
    }

    async fn list(&self, _: DeploymentId, realm: &ThemeRealm) -> Result<Vec<Theme>, CoreError> {
        Ok(self.themes_of(realm.as_str()))
    }

    async fn get(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
    ) -> Result<Option<Theme>, CoreError> {
        Ok(self
            .themes_of(realm.as_str())
            .into_iter()
            .find(|theme| theme.id == id))
    }

    async fn active(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Option<Theme>, CoreError> {
        Ok(self
            .themes_of(realm.as_str())
            .into_iter()
            .find(|theme| theme.active))
    }

    async fn insert(&self, theme: &Theme) -> Result<(), CoreError> {
        let mut held = self.0.lock().unwrap();
        if clash(&held, theme) {
            return Err(ThemeError::NameTaken {
                name: theme.name.as_str().to_string(),
            }
            .into());
        }
        held.themes.push(theme.clone());
        Ok(())
    }

    async fn update(&self, theme: &Theme) -> Result<(), CoreError> {
        let mut held = self.0.lock().unwrap();
        if clash(&held, theme) {
            return Err(ThemeError::NameTaken {
                name: theme.name.as_str().to_string(),
            }
            .into());
        }
        if let Some(stored) = held.themes.iter_mut().find(|stored| stored.id == theme.id) {
            stored.name = theme.name.clone();
            stored.tokens = theme.tokens.clone();
            stored.updated_at = theme.updated_at;
        }
        Ok(())
    }

    async fn delete(
        &self,
        _: DeploymentId,
        _: &ThemeRealm,
        id: ThemeId,
    ) -> Result<bool, CoreError> {
        let mut held = self.0.lock().unwrap();
        let before = held.themes.len();
        held.themes.retain(|theme| theme.id != id);
        Ok(held.themes.len() < before)
    }

    async fn activate(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
        id: ThemeId,
        _: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let mut held = self.0.lock().unwrap();
        if !held
            .themes
            .iter()
            .any(|theme| theme.id == id && theme.realm == *realm)
        {
            return Ok(false);
        }
        for theme in held.themes.iter_mut().filter(|theme| theme.realm == *realm) {
            theme.active = theme.id == id;
        }
        Ok(true)
    }

    async fn deactivate(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
        _: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mut held = self.0.lock().unwrap();
        for theme in held.themes.iter_mut().filter(|theme| theme.realm == *realm) {
            theme.active = false;
        }
        Ok(())
    }

    async fn next_revision(&self, _: DeploymentId, realm: &ThemeRealm) -> Result<u64, CoreError> {
        let mut held = self.0.lock().unwrap();
        let sync = held
            .syncs
            .entry(realm.as_str().to_string())
            .or_insert(ThemeSync {
                state: ThemeState::Pending,
                revision: 0,
            });
        sync.revision += 1;
        sync.state = ThemeState::Pending;
        Ok(sync.revision)
    }

    async fn sync(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
    ) -> Result<Option<ThemeSync>, CoreError> {
        Ok(self.sync_of(realm.as_str()))
    }

    async fn set_state(
        &self,
        _: DeploymentId,
        realm: &ThemeRealm,
        state: &ThemeState,
    ) -> Result<bool, CoreError> {
        let mut held = self.0.lock().unwrap();
        match held.syncs.get_mut(realm.as_str()) {
            Some(sync) => {
                sync.state = state.clone();
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

#[derive(Clone, Copy)]
struct Policy {
    manage: bool,
}

impl ThemePolicy for Policy {
    async fn can_view_themes(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
        Ok(())
    }

    async fn can_manage_themes(&self, _: Identity, _: OrganisationId) -> Result<(), CoreError> {
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
        resources: crate::dataplane::value_objects::DeploymentResources::DEFAULT,
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

fn subscription_opening(features: &[IamFeature]) -> Subscription {
    Subscription::open(
        OrganisationId(ORGANISATION),
        DeploymentId(DEPLOYMENT),
        SubscriptionTerms {
            tier: TierCode::parse("pro-2026").expect("a tier code"),
            monthly_price: Money::from_minor(4900),
            offer: Offer::Standard,
            features: features.iter().copied().collect::<BTreeSet<_>>(),
        },
        Utc::now(),
    )
}

fn subscriptions(latest: Option<Subscription>) -> MockSubscriptionRepository {
    let mut mock = MockSubscriptionRepository::new();
    mock.expect_latest_for().returning(move |_| {
        let latest = latest.clone();
        Box::pin(async move { Ok(latest) })
    });
    mock
}

#[derive(Clone, Default)]
struct Written {
    actions: Arc<Mutex<Vec<Action>>>,
}

impl Written {
    fn themes(&self) -> Vec<Action> {
        self.actions
            .lock()
            .unwrap()
            .iter()
            .filter(|action| action.action_type.0 == PORTAL_THEME_ACTION_TYPE)
            .cloned()
            .collect()
    }

    fn payloads(&self) -> Vec<Value> {
        self.themes()
            .into_iter()
            .map(|action| action.payload.data)
            .collect()
    }
}

type Service = ThemeServiceImpl<
    MockDeploymentRepository,
    Memory,
    Realms,
    MockActionRepository,
    MockSubscriptionRepository,
    Policy,
>;

struct World {
    service: Service,
    memory: Memory,
    written: Written,
}

fn world_with(
    kind: DeploymentKind,
    manage: bool,
    subscriptions: MockSubscriptionRepository,
) -> World {
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
        service: ThemeServiceImpl::new(
            deployments,
            memory.clone(),
            Realms(vec!["master", "customers"]),
            actions,
            subscriptions,
            Policy { manage },
        ),
        memory,
        written,
    }
}

fn ferriskey() -> World {
    world_with(
        DeploymentKind::Ferriskey,
        true,
        subscriptions(Some(subscription_opening(&[IamFeature::Branding]))),
    )
}

fn realm(name: &str) -> ThemeRealm {
    parse_realm(name).unwrap()
}

fn tokens(primary: &str) -> Value {
    json!({ "colors": { "primaryButton": primary } })
}

async fn create(
    world: &World,
    realm_name: &str,
    name: &str,
    tokens: Value,
) -> Result<Theme, CoreError> {
    world
        .service
        .create_theme(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm(realm_name),
            NewTheme::parse(name, &tokens).unwrap(),
        )
        .await
}

async fn made(world: &World, name: &str, primary: &str) -> Theme {
    create(world, "master", name, tokens(primary))
        .await
        .expect("created")
}

async fn change(
    world: &World,
    id: ThemeId,
    name: Option<&str>,
    tokens: Option<Value>,
) -> Result<Theme, CoreError> {
    world
        .service
        .update_theme(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm("master"),
            id,
            ThemeChange::parse(name, tokens.as_ref()).unwrap(),
        )
        .await
}

async fn delete(world: &World, id: ThemeId) -> Result<(), CoreError> {
    world
        .service
        .delete_theme(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm("master"),
            id,
        )
        .await
}

async fn activate_in(world: &World, realm_name: &str, id: ThemeId) -> Result<Theme, CoreError> {
    world
        .service
        .activate_theme(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm(realm_name),
            id,
        )
        .await
}

async fn activate(world: &World, id: ThemeId) -> Result<Theme, CoreError> {
    activate_in(world, "master", id).await
}

async fn deactivate(world: &World) -> Result<(), CoreError> {
    world
        .service
        .deactivate_themes(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm("master"),
        )
        .await
}

async fn list(world: &World) -> Result<ThemeListing, CoreError> {
    world
        .service
        .list_themes(
            caller(),
            OrganisationId(ORGANISATION),
            DeploymentId(DEPLOYMENT),
            realm("master"),
        )
        .await
}

fn is_denied(error: &CoreError) -> bool {
    matches!(error, CoreError::PermissionDenied { .. })
}

fn theme_error(error: CoreError) -> ThemeError {
    match error {
        CoreError::Theme(error) => error,
        other => panic!("expected a theme error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_created_theme_is_inactive_and_sends_nothing() {
    let world = ferriskey();

    let theme = made(&world, "Autumn", "#112233").await;

    assert!(!theme.active);
    assert_eq!(theme.name.as_str(), "Autumn");
    assert_eq!(world.memory.themes_of("master"), vec![theme]);
    assert!(world.written.themes().is_empty());
    assert!(world.memory.sync_of("master").is_none());
}

#[tokio::test]
async fn activating_sends_the_tokens_of_the_theme_with_a_first_revision_spec_br_6() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;

    let activated = activate(&world, theme.id).await.expect("activated");

    assert!(activated.active);
    assert!(world.memory.themes_of("master")[0].active);
    let sent = world.written.themes();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].version, ActionVersion(1));
    assert_eq!(sent[0].deployment_id, Some(DeploymentId(DEPLOYMENT)));
    assert_eq!(sent[0].dataplane_id, DataPlaneId(DATAPLANE));
    assert_eq!(sent[0].target.kind, TargetKind::Deployment);
    assert_eq!(sent[0].target.id, DEPLOYMENT);
    assert_eq!(
        sent[0].payload.data,
        json!({
            "deployment_id": DEPLOYMENT,
            "namespace": "production-auth",
            "revision": 1,
            "realm": "master",
            "config": { "colors": { "primaryButton": "#112233" } }
        })
    );
    assert_eq!(
        world.memory.sync_of("master"),
        Some(ThemeSync {
            state: ThemeState::Pending,
            revision: 1
        })
    );
}

#[tokio::test]
async fn activating_another_theme_replaces_the_active_one_with_a_higher_revision() {
    let world = ferriskey();
    let first = made(&world, "Autumn", "#112233").await;
    let second = made(&world, "Winter", "#445566").await;
    activate(&world, first.id).await.unwrap();

    activate(&world, second.id).await.unwrap();

    let active: Vec<_> = world
        .memory
        .themes_of("master")
        .into_iter()
        .filter(|theme| theme.active)
        .map(|theme| theme.id)
        .collect();
    assert_eq!(active, vec![second.id]);
    let payloads = world.written.payloads();
    assert_eq!(payloads[0]["revision"], 1);
    assert_eq!(payloads[1]["revision"], 2);
    assert_eq!(payloads[1]["config"], tokens("#445566"));
}

#[tokio::test]
async fn using_ferriskeys_default_sends_empty_tokens_spec_br_15() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();

    deactivate(&world).await.expect("deactivated");

    assert!(
        world
            .memory
            .themes_of("master")
            .iter()
            .all(|theme| !theme.active)
    );
    let payloads = world.written.payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[1]["config"], json!({}));
    assert_eq!(payloads[1]["revision"], 2);
}

#[tokio::test]
async fn saving_the_active_theme_sends_its_new_tokens_spec_br_7() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();

    let saved = change(&world, theme.id, None, Some(tokens("#abcdef")))
        .await
        .expect("saved");

    assert_eq!(
        saved.tokens,
        ThemeTokens::parse(&tokens("#abcdef")).unwrap()
    );
    assert!(saved.active);
    let payloads = world.written.payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[1]["revision"], 2);
    assert_eq!(payloads[1]["config"], tokens("#abcdef"));
}

#[tokio::test]
async fn saving_a_theme_that_is_not_active_sends_nothing_spec_br_7() {
    let world = ferriskey();
    let active = made(&world, "Autumn", "#112233").await;
    let other = made(&world, "Winter", "#445566").await;
    activate(&world, active.id).await.unwrap();

    let saved = change(&world, other.id, Some("Winter 2"), Some(tokens("#abcdef")))
        .await
        .expect("saved");

    assert_eq!(saved.name.as_str(), "Winter 2");
    assert_eq!(world.written.themes().len(), 1);
}

#[tokio::test]
async fn renaming_the_active_theme_or_saving_the_same_tokens_sends_nothing() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();

    change(&world, theme.id, Some("Fall"), None).await.unwrap();
    change(&world, theme.id, None, Some(tokens("#112233")))
        .await
        .unwrap();

    assert_eq!(world.memory.themes_of("master")[0].name.as_str(), "Fall");
    assert_eq!(world.written.themes().len(), 1);
}

#[tokio::test]
async fn deleting_a_theme_that_is_not_active_sends_nothing() {
    let world = ferriskey();
    let active = made(&world, "Autumn", "#112233").await;
    let other = made(&world, "Winter", "#445566").await;
    activate(&world, active.id).await.unwrap();

    delete(&world, other.id).await.expect("deleted");

    let kept: Vec<_> = world
        .memory
        .themes_of("master")
        .into_iter()
        .map(|theme| theme.id)
        .collect();
    assert_eq!(kept, vec![active.id]);
    assert_eq!(world.written.themes().len(), 1);
}

#[tokio::test]
async fn the_active_theme_cannot_be_deleted_spec_br_7() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();

    let refused = delete(&world, theme.id).await.expect_err("refused");

    assert_eq!(theme_error(refused), ThemeError::ActiveTheme);
    assert_eq!(world.memory.themes_of("master").len(), 1);
    assert_eq!(world.written.themes().len(), 1);
}

#[tokio::test]
async fn a_theme_can_be_deleted_once_deactivated() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();
    deactivate(&world).await.unwrap();

    delete(&world, theme.id).await.expect("deleted");

    assert!(world.memory.themes_of("master").is_empty());
}

#[tokio::test]
async fn a_realm_cannot_hold_an_eleventh_theme_spec_br_7() {
    let world = ferriskey();
    for index in 0..MAX_THEMES_PER_REALM {
        made(&world, &format!("Theme {index}"), "#112233").await;
    }

    let refused = create(&world, "master", "One too many", tokens("#112233"))
        .await
        .expect_err("refused");

    assert_eq!(theme_error(refused), ThemeError::ThemeLimit);
    assert_eq!(world.memory.themes_of("master").len(), MAX_THEMES_PER_REALM);
    create(&world, "customers", "Fresh", tokens("#112233"))
        .await
        .expect("another realm has its own ten");
}

#[tokio::test]
async fn a_name_is_unique_in_a_realm_whatever_its_case() {
    let world = ferriskey();
    let autumn = made(&world, "Autumn", "#112233").await;
    let winter = made(&world, "Winter", "#445566").await;

    let duplicate = create(&world, "master", "aUTUMN", tokens("#112233"))
        .await
        .expect_err("refused");
    let rename = change(&world, winter.id, Some("autumn"), None)
        .await
        .expect_err("refused");

    assert!(matches!(
        theme_error(duplicate),
        ThemeError::NameTaken { .. }
    ));
    assert!(matches!(theme_error(rename), ThemeError::NameTaken { .. }));
    create(&world, "customers", "Autumn", tokens("#112233"))
        .await
        .expect("the same name in another realm");
    change(&world, autumn.id, Some("AUTUMN"), None)
        .await
        .expect("a theme may change the case of its own name");
}

#[tokio::test]
async fn a_theme_is_found_only_in_its_own_realm() {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;

    let refused = activate_in(&world, "customers", theme.id)
        .await
        .expect_err("not found");

    assert!(matches!(theme_error(refused), ThemeError::NotFound { .. }));
    assert!(matches!(
        theme_error(delete(&world, ThemeId(Uuid::new_v4())).await.unwrap_err()),
        ThemeError::NotFound { .. }
    ));
    assert!(matches!(
        theme_error(
            change(&world, ThemeId(Uuid::new_v4()), Some("x"), None)
                .await
                .unwrap_err()
        ),
        ThemeError::NotFound { .. }
    ));
    assert!(world.written.themes().is_empty());
}

#[tokio::test]
async fn a_theme_in_a_realm_the_deployment_does_not_have_is_refused() {
    let world = ferriskey();

    let refused = create(&world, "nowhere", "Autumn", tokens("#112233"))
        .await
        .expect_err("refused");

    assert!(matches!(theme_error(refused), ThemeError::Invalid { .. }));
}

#[tokio::test]
async fn the_revision_counts_up_per_realm_and_never_goes_back_spec_br_11() {
    let world = ferriskey();
    let first = made(&world, "Autumn", "#112233").await;
    let second = made(&world, "Winter", "#445566").await;
    activate(&world, first.id).await.unwrap();
    activate(&world, second.id).await.unwrap();
    delete(&world, first.id).await.unwrap();
    deactivate(&world).await.unwrap();
    delete(&world, second.id).await.unwrap();
    let third = made(&world, "Spring", "#778899").await;
    activate(&world, third.id).await.unwrap();

    let revisions: Vec<u64> = world
        .written
        .payloads()
        .iter()
        .map(|payload| payload["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, vec![1, 2, 3, 4]);

    let other = create(&world, "customers", "Autumn", tokens("#112233"))
        .await
        .unwrap();
    activate_in(&world, "customers", other.id).await.unwrap();
    assert_eq!(world.written.payloads()[4]["revision"], 1);
    assert_eq!(world.written.payloads()[4]["realm"], "customers");
}

#[tokio::test]
async fn listing_gives_the_themes_and_the_state_of_what_is_applied() {
    let world = ferriskey();
    assert_eq!(list(&world).await.unwrap().state, None);
    let theme = made(&world, "Autumn", "#112233").await;
    assert_eq!(list(&world).await.unwrap().state, None);
    activate(&world, theme.id).await.unwrap();

    let listing = list(&world).await.unwrap();

    assert_eq!(listing.themes.len(), 1);
    assert_eq!(
        listing.state,
        Some(ThemeSync {
            state: ThemeState::Pending,
            revision: 1
        })
    );
}

#[tokio::test]
async fn a_viewer_lists_and_changes_nothing_spec_br_12() {
    let world = world_with(
        DeploymentKind::Ferriskey,
        false,
        subscriptions(Some(subscription_opening(&[IamFeature::Branding]))),
    );
    let id = ThemeId(Uuid::new_v4());

    assert!(list(&world).await.is_ok());
    assert!(is_denied(
        &create(&world, "master", "Autumn", tokens("#112233"))
            .await
            .unwrap_err()
    ));
    assert!(is_denied(
        &change(&world, id, Some("x"), None).await.unwrap_err()
    ));
    assert!(is_denied(&delete(&world, id).await.unwrap_err()));
    assert!(is_denied(&activate(&world, id).await.unwrap_err()));
    assert!(is_denied(&deactivate(&world).await.unwrap_err()));
    assert!(world.memory.themes_of("master").is_empty());
    assert!(world.written.themes().is_empty());
}

#[tokio::test]
async fn a_tier_without_branding_lists_but_changes_nothing_spec_br_13() {
    let world = world_with(
        DeploymentKind::Ferriskey,
        true,
        subscriptions(Some(subscription_opening(&[IamFeature::Mfa]))),
    );
    let id = ThemeId(Uuid::new_v4());

    assert!(list(&world).await.is_ok());
    let refused = create(&world, "master", "Autumn", tokens("#112233"))
        .await
        .unwrap_err();
    let CoreError::PermissionDenied { reason } = refused else {
        panic!("expected a permission refusal");
    };
    assert!(reason.contains("pro-2026"), "{reason}");
    assert!(reason.contains("branding"), "{reason}");
    assert!(is_denied(
        &change(&world, id, Some("x"), None).await.unwrap_err()
    ));
    assert!(is_denied(&delete(&world, id).await.unwrap_err()));
    assert!(is_denied(&activate(&world, id).await.unwrap_err()));
    assert!(is_denied(&deactivate(&world).await.unwrap_err()));
    assert!(world.memory.themes_of("master").is_empty());
    assert!(world.written.themes().is_empty());
}

#[tokio::test]
async fn a_deployment_without_a_subscription_changes_nothing() {
    let world = world_with(DeploymentKind::Ferriskey, true, subscriptions(None));

    let refused = create(&world, "master", "Autumn", tokens("#112233"))
        .await
        .unwrap_err();

    let CoreError::PermissionDenied { reason } = refused else {
        panic!("expected a permission refusal");
    };
    assert!(reason.contains("no subscription"), "{reason}");
    assert!(list(&world).await.is_ok());
}

#[tokio::test]
async fn another_kind_of_deployment_is_refused_whatever_its_plan() {
    let world = world_with(
        DeploymentKind::Keycloak,
        true,
        subscriptions(Some(subscription_opening(&[IamFeature::Branding]))),
    );
    let id = ThemeId(Uuid::new_v4());

    let unsupported = |error: CoreError| theme_error(error) == ThemeError::UnsupportedKind;
    assert!(unsupported(
        create(&world, "master", "Autumn", tokens("#112233"))
            .await
            .unwrap_err()
    ));
    assert!(unsupported(
        change(&world, id, Some("x"), None).await.unwrap_err()
    ));
    assert!(unsupported(delete(&world, id).await.unwrap_err()));
    assert!(unsupported(activate(&world, id).await.unwrap_err()));
    assert!(unsupported(deactivate(&world).await.unwrap_err()));
    assert!(list(&world).await.unwrap().themes.is_empty());
    assert!(world.written.themes().is_empty());
}

#[tokio::test]
async fn another_organisations_deployment_is_not_found() {
    let world = ferriskey();

    let refused = world
        .service
        .list_themes(
            caller(),
            OrganisationId(Uuid::from_u128(99)),
            DeploymentId(DEPLOYMENT),
            realm("master"),
        )
        .await
        .unwrap_err();

    assert!(matches!(refused, CoreError::DeploymentNotFound { .. }));
}

fn ingest_as(
    world: &World,
    speaking: Uuid,
) -> ThemeOutcomeServiceImpl<MockDeploymentRepository, MockDataPlaneRepository, Memory> {
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

    ThemeOutcomeServiceImpl::new(deployments, dataplanes, world.memory.clone())
}

fn outcome(revision: u64, realm_name: &str, phase: &str, message: Option<&str>) -> ThemeOutcome {
    ThemeOutcome::parse(
        revision,
        realm_name,
        vec![(phase.to_string(), message.map(str::to_string))],
    )
    .unwrap()
}

async fn report(world: &World, outcome: ThemeOutcome) -> bool {
    ingest_as(world, DATAPLANE)
        .report_theme_outcome(caller(), DeploymentId(DEPLOYMENT), outcome)
        .await
        .expect("reported")
}

async fn with_two_revisions() -> World {
    let world = ferriskey();
    let theme = made(&world, "Autumn", "#112233").await;
    activate(&world, theme.id).await.unwrap();
    deactivate(&world).await.unwrap();
    world
}

#[tokio::test]
async fn an_applied_report_of_the_current_revision_is_recorded() {
    let world = with_two_revisions().await;

    assert!(report(&world, outcome(2, "master", "Applied", None)).await);

    assert_eq!(
        world.memory.sync_of("master"),
        Some(ThemeSync {
            state: ThemeState::Applied,
            revision: 2
        })
    );
}

#[tokio::test]
async fn a_failed_report_keeps_its_message() {
    let world = with_two_revisions().await;

    assert!(
        report(
            &world,
            outcome(2, "master", "Failed", Some("404 portal themes"))
        )
        .await
    );

    assert_eq!(
        world.memory.sync_of("master").unwrap().state,
        ThemeState::Failed {
            message: "404 portal themes".to_string()
        }
    );
}

#[tokio::test]
async fn an_older_revision_redelivered_after_a_newer_one_changes_nothing_spec_br_11() {
    let world = with_two_revisions().await;
    assert!(report(&world, outcome(2, "master", "Applied", None)).await);

    assert!(!report(&world, outcome(1, "master", "Failed", Some("late"))).await);

    assert_eq!(
        world.memory.sync_of("master").unwrap().state,
        ThemeState::Applied
    );
}

#[tokio::test]
async fn a_report_of_an_older_revision_does_not_overwrite_a_pending_newer_one() {
    let world = with_two_revisions().await;

    assert!(!report(&world, outcome(1, "master", "Applied", None)).await);

    assert_eq!(
        world.memory.sync_of("master").unwrap().state,
        ThemeState::Pending
    );
}

#[tokio::test]
async fn a_new_action_puts_the_state_back_to_pending() {
    let world = with_two_revisions().await;
    report(&world, outcome(2, "master", "Applied", None)).await;
    let theme = made(&world, "Winter", "#445566").await;

    activate(&world, theme.id).await.unwrap();

    assert_eq!(
        world.memory.sync_of("master"),
        Some(ThemeSync {
            state: ThemeState::Pending,
            revision: 3
        })
    );
}

#[tokio::test]
async fn a_repeated_report_or_one_for_an_unknown_realm_changes_nothing() {
    let world = with_two_revisions().await;
    assert!(report(&world, outcome(2, "master", "Applied", None)).await);

    assert!(!report(&world, outcome(2, "master", "Applied", None)).await);
    assert!(!report(&world, outcome(1, "customers", "Applied", None)).await);
}

#[tokio::test]
async fn another_data_planes_report_is_refused() {
    let world = with_two_revisions().await;

    let refused = ingest_as(&world, Uuid::from_u128(77))
        .report_theme_outcome(
            caller(),
            DeploymentId(DEPLOYMENT),
            outcome(2, "master", "Applied", None),
        )
        .await;

    assert!(refused.is_err());
    assert_eq!(
        world.memory.sync_of("master").unwrap().state,
        ThemeState::Pending
    );
}
