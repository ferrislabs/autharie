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
    deployments::{DeploymentId, ports::DeploymentPolicy, ports::DeploymentRepository},
    offers::Offer,
    organisation::{OrganisationId, ports::OrganisationRepository},
    platform::{PlatformRight, ports::PlatformPolicy},
    quotas::{
        EffectiveQuota, OrganisationQuota, Quota, QuotaKind,
        ports::{DeploymentQuotaService, OrganisationQuotaService, QuotaChange, QuotaRepository},
        resolve,
    },
    user::ports::UserRepository,
};

/// The effective quota of one kind for a deployment on `offer`.
pub async fn effective<Q: QuotaRepository>(
    quotas: &Q,
    organisation_id: OrganisationId,
    offer: Option<Offer>,
    kind: QuotaKind,
) -> Result<Quota, CoreError> {
    let overrides = quotas.overrides(organisation_id).await?;

    Ok(resolve(overrides.get(&kind).copied(), offer, kind))
}

/// Refuses a create when the deployment already holds what its quota allows.
///
/// `held` is what the caller already reads to do the create, so this adds no
/// query of its own for the count. Only objects being created are refused;
/// lowering a quota below what is held deletes nothing.
pub async fn require_room<Q: QuotaRepository>(
    quotas: &Q,
    organisation_id: OrganisationId,
    offer: Option<Offer>,
    kind: QuotaKind,
    held: usize,
) -> Result<(), CoreError> {
    let quota = effective(quotas, organisation_id, offer, kind).await?;

    if quota.allows(held) {
        return Ok(());
    }

    let max = match quota {
        Quota::Limited(max) => max as usize,
        Quota::Unlimited => unreachable!("an unlimited quota allows everything"),
    };

    Err(CoreError::OrganisationLimitReached {
        limit_type: kind.to_string(),
        max,
        current: held,
    })
}

pub struct OrganisationQuotaServiceImpl<Q, P, O, A, U>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    quotas: Q,
    policy: P,
    organisations: O,
    audit: A,
    users: U,
}

impl<Q, P, O, A, U> OrganisationQuotaServiceImpl<Q, P, O, A, U>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    pub fn new(quotas: Q, policy: P, organisations: O, audit: A, users: U) -> Self {
        Self {
            quotas,
            policy,
            organisations,
            audit,
            users,
        }
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

    async fn view(
        &self,
        organisation_id: OrganisationId,
        kind: QuotaKind,
    ) -> Result<OrganisationQuota, CoreError> {
        let overrides = self.quotas.overrides(organisation_id).await?;

        Ok(OrganisationQuota::new(kind, overrides.get(&kind).copied()))
    }
}

impl<Q, P, O, A, U> OrganisationQuotaService for OrganisationQuotaServiceImpl<Q, P, O, A, U>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
{
    async fn list_organisation_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationQuota>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;
        self.require_organisation(organisation_id).await?;

        let overrides = self.quotas.overrides(organisation_id).await?;

        Ok(QuotaKind::ALL
            .into_iter()
            .map(|kind| OrganisationQuota::new(kind, overrides.get(&kind).copied()))
            .collect())
    }

    async fn set_organisation_quota(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
    ) -> Result<OrganisationQuota, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::ActOnTenant)
            .await?;
        self.require_organisation(organisation_id).await?;

        let before = self
            .quotas
            .overrides(organisation_id)
            .await?
            .get(&kind)
            .copied();
        let now = Utc::now();

        if before != quota {
            self.quotas
                .set_override(
                    organisation_id,
                    kind,
                    quota,
                    &QuotaChange {
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
                    AuditAction("organisation.quota.changed".to_string()),
                    AuditTarget {
                        kind: AuditTargetKind::Organisation,
                        id: organisation_id.0,
                    },
                    Some(AuditChange::new(
                        json!({ "quota": kind, "override": before }),
                        json!({ "quota": kind, "override": quota }),
                    )?),
                    now,
                ))
                .await?;
        }

        self.view(organisation_id, kind).await
    }
}

pub struct DeploymentQuotaServiceImpl<Q, D, P>
where
    Q: QuotaRepository,
    D: DeploymentRepository,
    P: DeploymentPolicy,
{
    quotas: Q,
    deployments: D,
    policy: P,
}

impl<Q, D, P> DeploymentQuotaServiceImpl<Q, D, P>
where
    Q: QuotaRepository,
    D: DeploymentRepository,
    P: DeploymentPolicy,
{
    pub fn new(quotas: Q, deployments: D, policy: P) -> Self {
        Self {
            quotas,
            deployments,
            policy,
        }
    }
}

impl<Q, D, P> DeploymentQuotaService for DeploymentQuotaServiceImpl<Q, D, P>
where
    Q: QuotaRepository,
    D: DeploymentRepository,
    P: DeploymentPolicy,
{
    async fn deployment_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        deployment_id: DeploymentId,
    ) -> Result<Vec<EffectiveQuota>, CoreError> {
        self.policy
            .can_view_deployments(identity, organisation_id)
            .await?;

        let deployment = self
            .deployments
            .get_by_id(deployment_id)
            .await?
            .filter(|deployment| deployment.organisation_id == organisation_id)
            .filter(|deployment| deployment.deleted_at.is_none())
            .ok_or(CoreError::DeploymentNotFound {
                id: deployment_id.0,
            })?;

        let overrides = self.quotas.overrides(organisation_id).await?;

        Ok(QuotaKind::ALL
            .into_iter()
            .map(|kind| {
                let overridden = overrides.get(&kind).copied();

                EffectiveQuota {
                    kind,
                    quota: resolve(overridden, deployment.offer, kind),
                    overridden: overridden.is_some(),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{
        audit::ports::MockAuditRepository,
        organisation::{
            Organisation,
            ports::MockOrganisationRepository,
            value_objects::{
                OrganisationLimits, OrganisationName, OrganisationSlug, OrganisationStatus, Plan,
            },
        },
        platform::PlatformRights,
        quotas::fixtures::InMemoryQuotas,
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
            id: org(),
            name: OrganisationName::new("Acme").expect("a name"),
            slug: OrganisationSlug::new("acme").expect("a slug"),
            owner_id: crate::user::UserId(Uuid::from_u128(2)),
            status: OrganisationStatus::Active,
            plan: Plan::Enterprise,
            limits: OrganisationLimits::from_plan(&Plan::Enterprise),
            created_at: now,
            updated_at: now,
            deleted_at: None,
            mcp_enabled: false,
        }
    }

    type Recorded = Arc<Mutex<Vec<AuditEntry>>>;

    type Service = OrganisationQuotaServiceImpl<
        InMemoryQuotas,
        Holding,
        MockOrganisationRepository,
        MockAuditRepository,
        NoUsers,
    >;

    fn service(quotas: InMemoryQuotas, holding: &[PlatformRight]) -> (Service, Recorded) {
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));

        let mut organisations = MockOrganisationRepository::new();
        organisations.expect_find_by_id().returning(|_| {
            let found = Some(organisation());
            Box::pin(async move { Ok(found) })
        });

        let mut audit = MockAuditRepository::new();
        let heard = Arc::clone(&recorded);
        audit.expect_append().returning(move |entry| {
            heard.lock().unwrap().push(entry);
            Box::pin(async { Ok(()) })
        });

        (
            OrganisationQuotaServiceImpl::new(
                quotas,
                Holding(PlatformRights::of(holding.iter().copied())),
                organisations,
                audit,
                NoUsers,
            ),
            recorded,
        )
    }

    #[tokio::test]
    async fn a_free_deployment_refuses_a_second_mcp_server() {
        let refused = require_room(
            &InMemoryQuotas::default(),
            org(),
            Some(Offer::Free),
            QuotaKind::McpServers,
            1,
        )
        .await
        .expect_err("the free offer holds one");

        let CoreError::OrganisationLimitReached {
            limit_type,
            max,
            current,
        } = refused
        else {
            panic!("the refusal was not a limit: {refused}");
        };
        assert_eq!((limit_type.as_str(), max, current), ("mcp_servers", 1, 1));
    }

    #[tokio::test]
    async fn the_first_object_is_accepted() {
        require_room(
            &InMemoryQuotas::default(),
            org(),
            Some(Offer::Free),
            QuotaKind::McpServers,
            0,
        )
        .await
        .expect("room for the first");
    }

    #[tokio::test]
    async fn an_override_raises_the_limit_whatever_the_offer() {
        let quotas =
            InMemoryQuotas::default().with(org(), QuotaKind::McpServers, Quota::Limited(3));

        require_room(&quotas, org(), Some(Offer::Free), QuotaKind::McpServers, 2)
            .await
            .expect("the third fits");
        assert!(
            require_room(&quotas, org(), Some(Offer::Free), QuotaKind::McpServers, 3)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_override_can_lower_the_limit_below_the_default() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(2));

        assert!(
            require_room(&quotas, org(), Some(Offer::Scale), QuotaKind::Policies, 2)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_override_of_zero_refuses_the_first() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(0));

        assert!(
            require_room(&quotas, org(), Some(Offer::Scale), QuotaKind::Policies, 0)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_unlimited_override_lifts_the_limit() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Unlimited);

        require_room(
            &quotas,
            org(),
            Some(Offer::Free),
            QuotaKind::Policies,
            10_000,
        )
        .await
        .expect("no limit");
    }

    #[tokio::test]
    async fn another_organisations_override_is_not_applied() {
        let other = OrganisationId(Uuid::from_u128(99));
        let quotas =
            InMemoryQuotas::default().with(other, QuotaKind::McpServers, Quota::Limited(9));

        assert!(
            require_room(&quotas, org(), Some(Offer::Free), QuotaKind::McpServers, 1)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_deployment_over_its_quota_keeps_what_it_holds() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(1));

        assert!(
            require_room(&quotas, org(), Some(Offer::Free), QuotaKind::Policies, 4)
                .await
                .is_err(),
            "only the next create is refused; nothing here deletes"
        );
    }

    #[tokio::test]
    async fn users_are_never_refused() {
        let quotas = InMemoryQuotas::default();

        for offer in Offer::ALL {
            require_room(&quotas, org(), Some(offer), QuotaKind::Users, 1_000_000)
                .await
                .expect("the users quota is not enforced");
        }
    }

    #[tokio::test]
    async fn listing_needs_view_estate() {
        let (service, _) = service(InMemoryQuotas::default(), &[PlatformRight::OperateFleet]);

        assert!(matches!(
            service.list_organisation_quotas(caller(), org()).await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
    }

    #[tokio::test]
    async fn listing_shows_every_kind_with_its_override() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(7));
        let (service, _) = service(quotas, &[PlatformRight::ViewEstate]);

        let listed = service
            .list_organisation_quotas(caller(), org())
            .await
            .unwrap();

        assert_eq!(listed.len(), QuotaKind::ALL.len());
        let policies = listed
            .iter()
            .find(|quota| quota.kind == QuotaKind::Policies)
            .unwrap();
        assert_eq!(policies.organisation_override, Some(Quota::Limited(7)));
        assert!(
            listed
                .iter()
                .filter(|quota| quota.kind != QuotaKind::Policies)
                .all(|quota| quota.organisation_override.is_none())
        );
    }

    #[tokio::test]
    async fn setting_needs_the_right_to_act_on_a_tenant() {
        let (service, recorded) = service(InMemoryQuotas::default(), &[PlatformRight::ViewEstate]);

        assert!(matches!(
            service
                .set_organisation_quota(
                    caller(),
                    org(),
                    QuotaKind::Policies,
                    Some(Quota::Limited(9))
                )
                .await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn setting_an_override_is_audited_with_the_old_and_new_value() {
        let (service, recorded) = service(InMemoryQuotas::default(), &[PlatformRight::ActOnTenant]);

        let view = service
            .set_organisation_quota(
                caller(),
                org(),
                QuotaKind::Policies,
                Some(Quota::Limited(20)),
            )
            .await
            .unwrap();

        assert_eq!(view.organisation_override, Some(Quota::Limited(20)));
        let entries = recorded.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action.0, "organisation.quota.changed");
    }

    #[tokio::test]
    async fn clearing_an_override_returns_to_the_default() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(20));
        let (service, recorded) = service(quotas.clone(), &[PlatformRight::ActOnTenant]);

        let view = service
            .set_organisation_quota(caller(), org(), QuotaKind::Policies, None)
            .await
            .unwrap();

        assert_eq!(view.organisation_override, None);
        assert!(quotas.overrides(org()).await.unwrap().is_empty());
        assert_eq!(recorded.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn setting_what_is_already_set_writes_nothing() {
        let quotas = InMemoryQuotas::default().with(org(), QuotaKind::Policies, Quota::Limited(20));
        let (service, recorded) = service(quotas, &[PlatformRight::ActOnTenant]);

        service
            .set_organisation_quota(
                caller(),
                org(),
                QuotaKind::Policies,
                Some(Quota::Limited(20)),
            )
            .await
            .unwrap();

        assert!(recorded.lock().unwrap().is_empty());
    }
}
