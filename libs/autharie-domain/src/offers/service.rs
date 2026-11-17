use autharie_auth::Identity;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::{
    CoreError,
    audit::{
        AuditAction, AuditChange, AuditEntry, AuditEntryId, AuditTarget, AuditTargetKind,
        ports::AuditRepository, service::audit_actor,
    },
    offers::{
        Offer, OrganisationOffer,
        ports::{OfferChange, OfferOverrideRepository, OrganisationOfferService},
    },
    organisation::{Organisation, OrganisationId, ports::OrganisationRepository},
    platform::{PlatformRight, ports::PlatformPolicy},
    user::ports::UserRepository,
};

pub struct OrganisationOfferServiceImpl<F, P, O, A, U>
where
    F: OfferOverrideRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    overrides: F,
    policy: P,
    organisations: O,
    audit: A,
    users: U,
}

impl<F, P, O, A, U> OrganisationOfferServiceImpl<F, P, O, A, U>
where
    F: OfferOverrideRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    pub fn new(overrides: F, policy: P, organisations: O, audit: A, users: U) -> Self {
        Self {
            overrides,
            policy,
            organisations,
            audit,
            users,
        }
    }

    async fn organisation(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<Organisation, CoreError> {
        self.organisations
            .find_by_id(&organisation_id)
            .await?
            .ok_or(CoreError::OrganisationNotFound {
                id: organisation_id.0,
            })
    }
}

impl<F, P, O, A, U> OrganisationOfferService for OrganisationOfferServiceImpl<F, P, O, A, U>
where
    F: OfferOverrideRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    async fn list_organisation_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationOffer>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;
        let organisation = self.organisation(organisation_id).await?;
        let overrides = self.overrides.overrides(organisation_id).await?;

        Ok(Offer::ALL
            .into_iter()
            .map(|offer| {
                OrganisationOffer::new(offer, organisation.plan, overrides.get(&offer).copied())
            })
            .collect())
    }

    async fn set_organisation_offer(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        offer: Offer,
        open: Option<bool>,
    ) -> Result<OrganisationOffer, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::ActOnTenant)
            .await?;
        let organisation = self.organisation(organisation_id).await?;

        let before = self
            .overrides
            .overrides(organisation_id)
            .await?
            .get(&offer)
            .copied();
        let now = Utc::now();

        if before != open {
            self.overrides
                .set_override(
                    organisation_id,
                    offer,
                    open,
                    &OfferChange {
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
                    AuditAction("organisation.offer.changed".to_string()),
                    AuditTarget {
                        kind: AuditTargetKind::Organisation,
                        id: organisation_id.0,
                    },
                    Some(AuditChange::new(
                        json!({ "offer": offer, "override": before }),
                        json!({ "offer": offer, "override": open }),
                    )?),
                    now,
                ))
                .await?;
        }

        Ok(OrganisationOffer::new(offer, organisation.plan, open))
    }
}

#[cfg(test)]
pub mod fixtures {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use crate::{
        CoreError,
        offers::{
            Offer,
            ports::{OfferChange, OfferOverrideRepository},
        },
        organisation::OrganisationId,
    };

    #[derive(Clone, Default)]
    pub struct InMemoryOffers {
        overrides: Arc<Mutex<HashMap<(OrganisationId, Offer), bool>>>,
    }

    impl InMemoryOffers {
        pub fn with(self, organisation_id: OrganisationId, offer: Offer, open: bool) -> Self {
            self.overrides
                .lock()
                .unwrap()
                .insert((organisation_id, offer), open);
            self
        }
    }

    impl OfferOverrideRepository for InMemoryOffers {
        async fn overrides(
            &self,
            organisation_id: OrganisationId,
        ) -> Result<HashMap<Offer, bool>, CoreError> {
            Ok(self
                .overrides
                .lock()
                .unwrap()
                .iter()
                .filter(|((organisation, _), _)| *organisation == organisation_id)
                .map(|((_, offer), open)| (*offer, *open))
                .collect())
        }

        async fn set_override(
            &self,
            organisation_id: OrganisationId,
            offer: Offer,
            open: Option<bool>,
            _change: &OfferChange,
        ) -> Result<(), CoreError> {
            let mut overrides = self.overrides.lock().unwrap();
            match open {
                Some(open) => overrides.insert((organisation_id, offer), open),
                None => overrides.remove(&(organisation_id, offer)),
            };
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{fixtures::InMemoryOffers, *};
    use crate::{
        audit::ports::MockAuditRepository,
        organisation::{
            ports::MockOrganisationRepository,
            value_objects::{
                OrganisationLimits, OrganisationName, OrganisationSlug, OrganisationStatus, Plan,
            },
        },
        platform::PlatformRights,
        user::{User, ports::UserRepository},
    };

    const ORGANISATION: Uuid = Uuid::from_u128(1);

    fn org() -> OrganisationId {
        OrganisationId(ORGANISATION)
    }

    struct NoUsers;

    impl UserRepository for NoUsers {
        async fn upsert_by_email(&self, _user: &User) -> Result<User, CoreError> {
            unreachable!("nothing here writes a user")
        }

        async fn find_by_sub(&self, _sub: &str) -> Result<Option<User>, CoreError> {
            unreachable!("the caller in these tests is a client")
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

    fn caller() -> Identity {
        Identity::Client(autharie_auth::Client {
            id: "me".to_string(),
            client_id: "somebody".to_string(),
            roles: vec![],
            scopes: vec![],
            act: None,
        })
    }

    type Recorded = Arc<Mutex<Vec<AuditEntry>>>;

    type Service = OrganisationOfferServiceImpl<
        InMemoryOffers,
        Holding,
        MockOrganisationRepository,
        MockAuditRepository,
        NoUsers,
    >;

    fn service(overrides: InMemoryOffers, holding: &[PlatformRight]) -> (Service, Recorded) {
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));

        let mut organisations = MockOrganisationRepository::new();
        organisations.expect_find_by_id().returning(|_| {
            let now = chrono::Utc::now();
            let found = Some(Organisation {
                id: org(),
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
            });
            Box::pin(async move { Ok(found) })
        });

        let mut audit = MockAuditRepository::new();
        let heard = Arc::clone(&recorded);
        audit.expect_append().returning(move |entry| {
            heard.lock().unwrap().push(entry);
            Box::pin(async { Ok(()) })
        });

        (
            OrganisationOfferServiceImpl::new(
                overrides,
                Holding(PlatformRights::of(holding.iter().copied())),
                organisations,
                audit,
                NoUsers,
            ),
            recorded,
        )
    }

    #[tokio::test]
    async fn listing_needs_view_estate() {
        let (service, _) = service(InMemoryOffers::default(), &[PlatformRight::OperateFleet]);

        assert!(matches!(
            service.list_organisation_offers(caller(), org()).await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
    }

    #[tokio::test]
    async fn listing_shows_every_offer_with_its_default_override_and_effective() {
        let overrides = InMemoryOffers::default().with(org(), Offer::Sandbox, true);
        let (service, _) = service(overrides, &[PlatformRight::ViewEstate]);

        let listed = service
            .list_organisation_offers(caller(), org())
            .await
            .unwrap();

        assert_eq!(listed.len(), Offer::ALL.len());
        let sandbox = listed
            .iter()
            .find(|entry| entry.offer == Offer::Sandbox)
            .unwrap();
        assert_eq!(
            (
                sandbox.plan_default,
                sandbox.organisation_override,
                sandbox.effective
            ),
            (false, Some(true), true)
        );
        let free = listed
            .iter()
            .find(|entry| entry.offer == Offer::Free)
            .unwrap();
        assert_eq!(
            (
                free.plan_default,
                free.organisation_override,
                free.effective
            ),
            (true, None, true)
        );
    }

    #[tokio::test]
    async fn setting_needs_the_right_to_act_on_a_tenant() {
        let (service, recorded) = service(InMemoryOffers::default(), &[PlatformRight::ViewEstate]);

        assert!(matches!(
            service
                .set_organisation_offer(caller(), org(), Offer::Sandbox, Some(true))
                .await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn opening_an_offer_is_audited_and_clearing_returns_to_the_plan() {
        let overrides = InMemoryOffers::default();
        let (service, recorded) = service(overrides.clone(), &[PlatformRight::ActOnTenant]);

        let opened = service
            .set_organisation_offer(caller(), org(), Offer::Sandbox, Some(true))
            .await
            .unwrap();
        assert!(opened.effective);

        let cleared = service
            .set_organisation_offer(caller(), org(), Offer::Sandbox, None)
            .await
            .unwrap();
        assert!(!cleared.effective);
        assert!(overrides.overrides(org()).await.unwrap().is_empty());

        let entries = recorded.lock().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.action.0 == "organisation.offer.changed")
        );
    }

    #[tokio::test]
    async fn setting_what_is_already_set_writes_nothing() {
        let overrides = InMemoryOffers::default().with(org(), Offer::Sandbox, true);
        let (service, recorded) = service(overrides, &[PlatformRight::ActOnTenant]);

        service
            .set_organisation_offer(caller(), org(), Offer::Sandbox, Some(true))
            .await
            .unwrap();

        assert!(recorded.lock().unwrap().is_empty());
    }
}
