use autharie_auth::Identity;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::{
    CoreError,
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        fleet::{
            FleetAuditAction, FleetAuditEntry, FleetAuditEntryId, FleetTarget,
            ports::FleetAuditRepository, service::fleet_actor,
        },
        ports::AuditRepository,
        service::audit_actor,
    },
    deployments::ports::DeploymentPolicy,
    features::{
        Feature, FeatureOverview, OrganisationFeature,
        ports::{FeatureFlagService, FeatureRepository, FlagChange, OrganisationFeatureService},
    },
    generate_uuid_v7,
    organisation::{OrganisationId, ports::OrganisationRepository},
    platform::{PlatformRight, ports::PlatformPolicy},
    user::ports::UserRepository,
};

pub async fn require_enabled<F: FeatureRepository>(
    features: &F,
    organisation_id: OrganisationId,
    feature: Feature,
) -> Result<(), CoreError> {
    if features.is_enabled(organisation_id, feature).await? {
        return Ok(());
    }

    Err(CoreError::FeatureNotEnabled {
        feature: feature.to_string(),
    })
}

pub struct FeatureFlagServiceImpl<F, P, O, A, U, FA>
where
    F: FeatureRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    FA: FleetAuditRepository,
{
    features: F,
    policy: P,
    organisations: O,
    audit: A,
    users: U,
    fleet_audit: FA,
}

impl<F, P, O, A, U, FA> FeatureFlagServiceImpl<F, P, O, A, U, FA>
where
    F: FeatureRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    FA: FleetAuditRepository,
{
    pub fn new(
        features: F,
        policy: P,
        organisations: O,
        audit: A,
        users: U,
        fleet_audit: FA,
    ) -> Self {
        Self {
            features,
            policy,
            organisations,
            audit,
            users,
            fleet_audit,
        }
    }

    async fn overview(&self, feature: Feature) -> Result<FeatureOverview, CoreError> {
        Ok(self
            .overviews()
            .await?
            .into_iter()
            .find(|overview| overview.feature == feature)
            .unwrap_or(FeatureOverview {
                feature,
                enabled_by_default: false,
                overrides: Default::default(),
            }))
    }

    async fn overviews(&self) -> Result<Vec<FeatureOverview>, CoreError> {
        let defaults = self.features.defaults().await?;
        let tallies = self.features.override_tallies().await?;

        Ok(Feature::ALL
            .into_iter()
            .map(|feature| FeatureOverview {
                feature,
                enabled_by_default: defaults.get(&feature).copied().unwrap_or(false),
                overrides: tallies.get(&feature).copied().unwrap_or_default(),
            })
            .collect())
    }

    async fn organisation_features(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationFeature>, CoreError> {
        let defaults = self.features.defaults().await?;
        let overrides = self.features.overrides(organisation_id).await?;

        Ok(Feature::ALL
            .into_iter()
            .map(|feature| {
                OrganisationFeature::new(
                    feature,
                    defaults.get(&feature).copied().unwrap_or(false),
                    overrides.get(&feature).copied(),
                )
            })
            .collect())
    }

    async fn organisation_feature(
        &self,
        organisation_id: OrganisationId,
        feature: Feature,
    ) -> Result<OrganisationFeature, CoreError> {
        Ok(self
            .organisation_features(organisation_id)
            .await?
            .into_iter()
            .find(|view| view.feature == feature)
            .unwrap_or(OrganisationFeature::new(feature, false, None)))
    }

    async fn require_organisation(&self, organisation_id: OrganisationId) -> Result<(), CoreError> {
        self.organisations
            .find_by_id(&organisation_id)
            .await?
            .map(|_| ())
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })
    }
}

impl<F, P, O, A, U, FA> FeatureFlagService for FeatureFlagServiceImpl<F, P, O, A, U, FA>
where
    F: FeatureRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    FA: FleetAuditRepository,
{
    async fn list_features(&self, identity: Identity) -> Result<Vec<FeatureOverview>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;

        self.overviews().await
    }

    async fn set_feature_default(
        &self,
        identity: Identity,
        feature: Feature,
        enabled: bool,
    ) -> Result<FeatureOverview, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::OperateFleet)
            .await?;

        let before = self
            .features
            .defaults()
            .await?
            .get(&feature)
            .copied()
            .unwrap_or(false);
        let now = Utc::now();

        self.features
            .set_default(
                feature,
                enabled,
                &FlagChange {
                    by: identity.id().to_string(),
                    at: now,
                },
            )
            .await?;

        if before != enabled {
            self.fleet_audit
                .append(FleetAuditEntry::record(
                    FleetAuditEntryId(generate_uuid_v7()),
                    fleet_actor(&identity),
                    FleetAuditAction::FeatureDefaultChanged,
                    FleetTarget::Feature {
                        key: feature.to_string(),
                    },
                    Some(AuditChange::new(
                        json!({ "enabled_by_default": before }),
                        json!({ "enabled_by_default": enabled }),
                    )?),
                    now,
                ))
                .await?;
        }

        self.overview(feature).await
    }

    async fn list_organisation_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationFeature>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;
        self.require_organisation(organisation_id).await?;

        self.organisation_features(organisation_id).await
    }

    async fn set_organisation_feature(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        feature: Feature,
        enabled: Option<bool>,
    ) -> Result<OrganisationFeature, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::ActOnTenant)
            .await?;
        self.require_organisation(organisation_id).await?;

        let before = self
            .features
            .overrides(organisation_id)
            .await?
            .get(&feature)
            .copied();
        let now = Utc::now();

        if before != enabled {
            self.features
                .set_override(
                    organisation_id,
                    feature,
                    enabled,
                    &FlagChange {
                        by: identity.id().to_string(),
                        at: now,
                    },
                )
                .await?;

            self.audit
                .append(AuditEntry::record(
                    AuditEntryId(Uuid::new_v4()),
                    organisation_id,
                    audit_actor(&identity, &self.users).await?,
                    AuditAction("organisation.feature.changed".to_string()),
                    AuditTarget {
                        kind: AuditTargetKind::Organisation,
                        id: organisation_id.0,
                    },
                    Some(AuditChange::new(
                        json!({ "feature": feature, "override": before }),
                        json!({ "feature": feature, "override": enabled }),
                    )?),
                    now,
                ))
                .await?;
        }

        self.organisation_feature(organisation_id, feature).await
    }
}

pub struct OrganisationFeatureServiceImpl<F, D>
where
    F: FeatureRepository,
    D: DeploymentPolicy,
{
    features: F,
    policy: D,
}

impl<F, D> OrganisationFeatureServiceImpl<F, D>
where
    F: FeatureRepository,
    D: DeploymentPolicy,
{
    pub fn new(features: F, policy: D) -> Self {
        Self { features, policy }
    }
}

impl<F, D> OrganisationFeatureService for OrganisationFeatureServiceImpl<F, D>
where
    F: FeatureRepository,
    D: DeploymentPolicy,
{
    async fn enabled_features(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Feature>, CoreError> {
        self.policy
            .can_view_deployments(identity, organisation_id)
            .await?;

        self.features.enabled_for(organisation_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{
        audit::{AuditEntry, fleet::fixtures::Recording, ports::MockAuditRepository},
        features::OverrideTally,
        organisation::{
            Organisation,
            ports::MockOrganisationRepository,
            value_objects::{
                OrganisationLimits, OrganisationName, OrganisationSlug, OrganisationStatus, Plan,
            },
        },
        platform::PlatformRights,
        user::{User, ports::UserRepository},
    };

    const ORGANISATION: Uuid = Uuid::from_u128(1);

    #[derive(Clone, Default)]
    struct InMemoryFeatures {
        defaults: Arc<Mutex<HashMap<Feature, bool>>>,
        overrides: Arc<Mutex<HashMap<(OrganisationId, Feature), bool>>>,
    }

    impl FeatureRepository for InMemoryFeatures {
        async fn defaults(&self) -> Result<HashMap<Feature, bool>, CoreError> {
            Ok(self.defaults.lock().unwrap().clone())
        }

        async fn set_default(
            &self,
            feature: Feature,
            enabled: bool,
            _change: &FlagChange,
        ) -> Result<(), CoreError> {
            self.defaults.lock().unwrap().insert(feature, enabled);
            Ok(())
        }

        async fn overrides(
            &self,
            organisation_id: OrganisationId,
        ) -> Result<HashMap<Feature, bool>, CoreError> {
            Ok(self
                .overrides
                .lock()
                .unwrap()
                .iter()
                .filter(|((organisation, _), _)| *organisation == organisation_id)
                .map(|((_, feature), enabled)| (*feature, *enabled))
                .collect())
        }

        async fn set_override(
            &self,
            organisation_id: OrganisationId,
            feature: Feature,
            enabled: Option<bool>,
            _change: &FlagChange,
        ) -> Result<(), CoreError> {
            let mut overrides = self.overrides.lock().unwrap();
            match enabled {
                Some(enabled) => overrides.insert((organisation_id, feature), enabled),
                None => overrides.remove(&(organisation_id, feature)),
            };
            Ok(())
        }

        async fn override_tallies(&self) -> Result<HashMap<Feature, OverrideTally>, CoreError> {
            let mut tallies: HashMap<Feature, OverrideTally> = HashMap::new();
            for ((_, feature), enabled) in self.overrides.lock().unwrap().iter() {
                let tally = tallies.entry(*feature).or_default();
                if *enabled {
                    tally.on += 1;
                } else {
                    tally.off += 1;
                }
            }
            Ok(tallies)
        }
    }

    struct NoUsers;

    impl UserRepository for NoUsers {
        async fn upsert_by_email(&self, _user: &User) -> Result<User, CoreError> {
            unreachable!("nothing here writes a user")
        }

        async fn find_by_sub(&self, _sub: &str) -> Result<Option<User>, CoreError> {
            unreachable!("the caller in these tests is a client, not a person")
        }

        async fn find_by_email(&self, _email: &str) -> Result<Option<User>, CoreError> {
            unreachable!("nothing here looks a user up by email")
        }
    }

    struct Holding(PlatformRights);

    impl PlatformPolicy for Holding {
        async fn require(
            &self,
            _identity: Identity,
            right: PlatformRight,
        ) -> Result<(), CoreError> {
            if self.0.holds(right) {
                return Ok(());
            }

            Err(CoreError::MissingPlatformRight {
                right: right.to_string(),
            })
        }

        async fn rights_of(&self, _identity: Identity) -> Result<PlatformRights, CoreError> {
            Ok(self.0.clone())
        }
    }

    struct Member(bool);

    impl DeploymentPolicy for Member {
        async fn can_view_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            if self.0 {
                return Ok(());
            }

            Err(CoreError::PermissionDenied {
                reason: "insufficient permissions".to_string(),
            })
        }

        async fn can_create_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("flags are only read")
        }

        async fn can_manage_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("flags are only read")
        }

        async fn can_delete_deployments(
            &self,
            _identity: Identity,
            _organisation_id: OrganisationId,
        ) -> Result<(), CoreError> {
            unreachable!("flags are only read")
        }
    }

    fn caller() -> Identity {
        Identity::Client(autharie_auth::Client {
            id: "me".to_string(),
            client_id: "somebody".to_string(),
            roles: vec![],
            scopes: vec![],
            act: None,
        })
    }

    fn organisation() -> Organisation {
        let now = Utc::now();

        Organisation {
            id: OrganisationId(ORGANISATION),
            name: OrganisationName::new("Acme").expect("a name"),
            slug: OrganisationSlug::new("acme").expect("a slug"),
            owner_id: crate::user::UserId(Uuid::from_u128(2)),
            status: OrganisationStatus::Active,
            plan: Plan::Free,
            limits: OrganisationLimits::from_plan(&Plan::Free),
            created_at: now,
            updated_at: now,
            deleted_at: None,
            mcp_enabled: false,
        }
    }

    type Recorded = Arc<Mutex<Vec<AuditEntry>>>;

    type Service = FeatureFlagServiceImpl<
        InMemoryFeatures,
        Holding,
        MockOrganisationRepository,
        MockAuditRepository,
        NoUsers,
        Recording,
    >;

    fn service(
        features: InMemoryFeatures,
        holding: PlatformRights,
        known: bool,
    ) -> (Service, Recorded, Recording) {
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
        let fleet = Recording::new();

        let mut organisations = MockOrganisationRepository::new();
        organisations.expect_find_by_id().returning(move |_| {
            let found = known.then(organisation);
            Box::pin(async move { Ok(found) })
        });

        let mut audit = MockAuditRepository::new();
        let heard = Arc::clone(&recorded);
        audit.expect_append().returning(move |entry| {
            heard.lock().unwrap().push(entry);
            Box::pin(async { Ok(()) })
        });

        (
            FeatureFlagServiceImpl::new(
                features,
                Holding(holding),
                organisations,
                audit,
                NoUsers,
                fleet.clone(),
            ),
            recorded,
            fleet,
        )
    }

    fn rights(of: PlatformRight) -> PlatformRights {
        PlatformRights::of([of])
    }

    #[tokio::test]
    async fn the_gate_refuses_a_feature_nobody_switched_on() {
        let refused = require_enabled(
            &InMemoryFeatures::default(),
            OrganisationId(ORGANISATION),
            Feature::CustomerCloud,
        )
        .await
        .expect_err("a feature nobody enabled");

        let CoreError::FeatureNotEnabled { feature } = refused else {
            panic!("the refusal did not name the feature: {refused}");
        };
        assert_eq!(feature, "customer_cloud");
    }

    #[tokio::test]
    async fn the_gate_opens_on_the_default() {
        let features = InMemoryFeatures::default();
        features
            .defaults
            .lock()
            .unwrap()
            .insert(Feature::CustomerCloud, true);

        require_enabled(
            &features,
            OrganisationId(ORGANISATION),
            Feature::CustomerCloud,
        )
        .await
        .expect("enabled by default");
    }

    #[tokio::test]
    async fn the_gate_closes_on_an_override_against_the_default() {
        let features = InMemoryFeatures::default();
        features
            .defaults
            .lock()
            .unwrap()
            .insert(Feature::CustomerCloud, true);
        features.overrides.lock().unwrap().insert(
            (OrganisationId(ORGANISATION), Feature::CustomerCloud),
            false,
        );

        assert!(
            require_enabled(
                &features,
                OrganisationId(ORGANISATION),
                Feature::CustomerCloud
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn listing_needs_view_estate() {
        let (service, _, _) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::OperateFleet),
            true,
        );

        let refused = service.list_features(caller()).await.expect_err("refused");

        assert!(matches!(
            refused,
            CoreError::MissingPlatformRight { ref right } if right == "view_estate"
        ));
    }

    #[tokio::test]
    async fn listing_shows_every_feature_with_its_tally() {
        let features = InMemoryFeatures::default();
        features
            .overrides
            .lock()
            .unwrap()
            .insert((OrganisationId(ORGANISATION), Feature::CustomerCloud), true);
        let (service, _, _) = service(features, rights(PlatformRight::ViewEstate), true);

        let listed = service.list_features(caller()).await.expect("listed");

        assert_eq!(listed.len(), Feature::ALL.len());
        assert_eq!(listed[0].overrides, OverrideTally { on: 1, off: 0 });
        assert!(!listed[0].enabled_by_default);
    }

    #[tokio::test]
    async fn changing_the_default_needs_operate_fleet() {
        let features = InMemoryFeatures::default();
        let (service, _, fleet) =
            service(features.clone(), rights(PlatformRight::ViewEstate), true);

        let refused = service
            .set_feature_default(caller(), Feature::CustomerCloud, true)
            .await
            .expect_err("a reader changed a default");

        assert!(matches!(
            refused,
            CoreError::MissingPlatformRight { ref right } if right == "operate_fleet"
        ));
        assert!(features.defaults.lock().unwrap().is_empty());
        assert!(fleet.entries().is_empty());
    }

    #[tokio::test]
    async fn changing_the_default_is_recorded_in_the_fleet_trail() {
        let (service, _, fleet) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::OperateFleet),
            true,
        );

        let overview = service
            .set_feature_default(caller(), Feature::CustomerCloud, true)
            .await
            .expect("changed");

        assert!(overview.enabled_by_default);
        let entry = fleet.only();
        assert_eq!(entry.action, FleetAuditAction::FeatureDefaultChanged);
        assert_eq!(
            entry.target,
            FleetTarget::Feature {
                key: "customer_cloud".to_string()
            }
        );
        let change = entry.change.expect("a change");
        assert_eq!(change.before(), &json!({ "enabled_by_default": false }));
        assert_eq!(change.after(), &json!({ "enabled_by_default": true }));
    }

    #[tokio::test]
    async fn setting_the_default_it_already_has_records_nothing() {
        let (service, _, fleet) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::OperateFleet),
            true,
        );

        service
            .set_feature_default(caller(), Feature::CustomerCloud, false)
            .await
            .expect("accepted");

        assert!(fleet.entries().is_empty());
    }

    #[tokio::test]
    async fn an_override_needs_act_on_tenant() {
        let features = InMemoryFeatures::default();
        let (service, recorded, _) =
            service(features.clone(), rights(PlatformRight::OperateFleet), true);

        let refused = service
            .set_organisation_feature(
                caller(),
                OrganisationId(ORGANISATION),
                Feature::CustomerCloud,
                Some(true),
            )
            .await
            .expect_err("refused");

        assert!(matches!(
            refused,
            CoreError::MissingPlatformRight { ref right } if right == "act_on_tenant"
        ));
        assert!(features.overrides.lock().unwrap().is_empty());
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_override_for_an_organisation_nobody_has_is_not_found() {
        let (service, _, _) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::ActOnTenant),
            false,
        );

        let refused = service
            .set_organisation_feature(
                caller(),
                OrganisationId(ORGANISATION),
                Feature::CustomerCloud,
                Some(true),
            )
            .await
            .expect_err("no such organisation");

        assert!(matches!(refused, CoreError::OrganisationNotFound { .. }));
    }

    #[tokio::test]
    async fn setting_then_clearing_an_override_goes_back_to_the_default() {
        let (service, recorded, _) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::ActOnTenant),
            true,
        );

        let on = service
            .set_organisation_feature(
                caller(),
                OrganisationId(ORGANISATION),
                Feature::CustomerCloud,
                Some(true),
            )
            .await
            .expect("set");
        assert_eq!(on.organisation_override, Some(true));
        assert!(on.effective);

        let cleared = service
            .set_organisation_feature(
                caller(),
                OrganisationId(ORGANISATION),
                Feature::CustomerCloud,
                None,
            )
            .await
            .expect("cleared");
        assert_eq!(cleared.organisation_override, None);
        assert!(!cleared.effective);

        assert_eq!(recorded.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn repeating_an_override_records_nothing_more() {
        let (service, recorded, _) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::ActOnTenant),
            true,
        );

        for _ in 0..2 {
            service
                .set_organisation_feature(
                    caller(),
                    OrganisationId(ORGANISATION),
                    Feature::CustomerCloud,
                    Some(true),
                )
                .await
                .expect("set");
        }

        assert_eq!(recorded.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_organisation_listing_resolves_each_feature() {
        let features = InMemoryFeatures::default();
        features
            .defaults
            .lock()
            .unwrap()
            .insert(Feature::CustomerCloud, true);
        features.overrides.lock().unwrap().insert(
            (OrganisationId(ORGANISATION), Feature::CustomerCloud),
            false,
        );
        let (service, _, _) = service(features, rights(PlatformRight::ViewEstate), true);

        let listed = service
            .list_organisation_features(caller(), OrganisationId(ORGANISATION))
            .await
            .expect("listed");

        assert!(listed[0].enabled_by_default);
        assert_eq!(listed[0].organisation_override, Some(false));
        assert!(!listed[0].effective);
    }

    #[tokio::test]
    async fn the_organisation_listing_for_an_unknown_organisation_is_not_found() {
        let (service, _, _) = service(
            InMemoryFeatures::default(),
            rights(PlatformRight::ViewEstate),
            false,
        );

        let refused = service
            .list_organisation_features(caller(), OrganisationId(ORGANISATION))
            .await
            .expect_err("no such organisation");

        assert!(matches!(refused, CoreError::OrganisationNotFound { .. }));
    }

    #[tokio::test]
    async fn a_member_sees_only_what_is_effectively_on() {
        let features = InMemoryFeatures::default();
        features
            .overrides
            .lock()
            .unwrap()
            .insert((OrganisationId(ORGANISATION), Feature::CustomerCloud), true);

        let enabled = OrganisationFeatureServiceImpl::new(features, Member(true))
            .enabled_features(caller(), OrganisationId(ORGANISATION))
            .await
            .expect("listed");

        assert_eq!(enabled, vec![Feature::CustomerCloud]);
    }

    #[tokio::test]
    async fn somebody_outside_the_organisation_sees_nothing() {
        let refused =
            OrganisationFeatureServiceImpl::new(InMemoryFeatures::default(), Member(false))
                .enabled_features(caller(), OrganisationId(ORGANISATION))
                .await
                .expect_err("not a member");

        assert!(matches!(refused, CoreError::PermissionDenied { .. }));
    }
}
