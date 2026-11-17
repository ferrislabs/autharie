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
    organisation::{
        Organisation, OrganisationId,
        invitation::InvitationState,
        ports::{InvitationRepository, OrganisationRepository},
        value_objects::Plan,
    },
    platform::{PlatformRight, ports::PlatformPolicy},
    quotas::{
        EffectiveQuota, OrganisationQuota, OrganisationQuotaKind, OrganisationScopeQuota, Quota,
        QuotaKind,
        ports::{
            DeploymentQuotaService, OrganisationQuotaService, OrganisationScopeQuotaService,
            QuotaChange, QuotaRepository,
        },
        resolve, resolve_for_plan,
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

    refuse_past(quota, kind.as_str(), held)
}

/// Refuses a create when the organisation already holds what its quota allows.
pub async fn require_organisation_room<Q: QuotaRepository>(
    quotas: &Q,
    organisation_id: OrganisationId,
    plan: Plan,
    kind: OrganisationQuotaKind,
    held: usize,
) -> Result<(), CoreError> {
    let overrides = quotas.organisation_overrides(organisation_id).await?;
    let quota = resolve_for_plan(overrides.get(&kind).copied(), plan, kind);

    refuse_past(quota, kind.as_str(), held)
}

fn refuse_past(quota: Quota, limit_type: &str, held: usize) -> Result<(), CoreError> {
    if quota.allows(held) {
        return Ok(());
    }

    let max = match quota {
        Quota::Limited(max) => max as usize,
        Quota::Unlimited => unreachable!("an unlimited quota allows everything"),
    };

    Err(CoreError::OrganisationLimitReached {
        limit_type: limit_type.to_string(),
        max,
        current: held,
    })
}

/// Deployments of the organisation that are not deleted, whatever their offer.
pub async fn live_deployments<D: DeploymentRepository>(
    deployments: &D,
    organisation_id: OrganisationId,
) -> Result<usize, CoreError> {
    Ok(deployments
        .list_by_organisation(organisation_id)
        .await?
        .iter()
        .filter(|deployment| deployment.deleted_at.is_none())
        .count())
}

/// Invitations that can still be walked through: not accepted, revoked or
/// expired.
pub async fn pending_invitations<I: InvitationRepository>(
    invitations: &I,
    organisation_id: OrganisationId,
) -> Result<usize, CoreError> {
    let now = Utc::now();

    Ok(invitations
        .list_invitations(&organisation_id)
        .await?
        .iter()
        .filter(|invitation| invitation.state(now) == InvitationState::Pending)
        .count())
}

pub async fn members_of<O: OrganisationRepository>(
    organisations: &O,
    organisation_id: OrganisationId,
) -> Result<usize, CoreError> {
    Ok(organisations.list_members(&organisation_id).await?.len())
}

pub struct OrganisationScopeQuotaServiceImpl<Q, P, O, A, U, D, I>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    D: DeploymentRepository,
    I: InvitationRepository,
{
    quotas: Q,
    policy: P,
    organisations: O,
    audit: A,
    users: U,
    deployments: D,
    invitations: I,
}

impl<Q, P, O, A, U, D, I> OrganisationScopeQuotaServiceImpl<Q, P, O, A, U, D, I>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    D: DeploymentRepository,
    I: InvitationRepository,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        quotas: Q,
        policy: P,
        organisations: O,
        audit: A,
        users: U,
        deployments: D,
        invitations: I,
    ) -> Self {
        Self {
            quotas,
            policy,
            organisations,
            audit,
            users,
            deployments,
            invitations,
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

    async fn held(
        &self,
        organisation_id: OrganisationId,
        kind: OrganisationQuotaKind,
    ) -> Result<usize, CoreError> {
        match kind {
            OrganisationQuotaKind::Deployments => {
                live_deployments(&self.deployments, organisation_id).await
            }
            OrganisationQuotaKind::Members => Ok(members_of(&self.organisations, organisation_id)
                .await?
                + pending_invitations(&self.invitations, organisation_id).await?),
        }
    }

    async fn view(
        &self,
        organisation: &Organisation,
        kind: OrganisationQuotaKind,
    ) -> Result<OrganisationScopeQuota, CoreError> {
        let overrides = self.quotas.organisation_overrides(organisation.id).await?;

        Ok(OrganisationScopeQuota::new(
            kind,
            organisation.plan,
            overrides.get(&kind).copied(),
            self.held(organisation.id, kind).await?,
        ))
    }
}

impl<Q, P, O, A, U, D, I> OrganisationScopeQuotaService
    for OrganisationScopeQuotaServiceImpl<Q, P, O, A, U, D, I>
where
    Q: QuotaRepository,
    P: PlatformPolicy,
    O: OrganisationRepository,
    A: AuditRepository,
    U: UserRepository,
    D: DeploymentRepository,
    I: InvitationRepository,
{
    async fn list_organisation_scope_quotas(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationScopeQuota>, CoreError> {
        self.policy
            .require(identity, PlatformRight::ViewEstate)
            .await?;
        let organisation = self.organisation(organisation_id).await?;

        let mut listed = Vec::with_capacity(OrganisationQuotaKind::ALL.len());
        for kind in OrganisationQuotaKind::ALL {
            listed.push(self.view(&organisation, kind).await?);
        }

        Ok(listed)
    }

    async fn set_organisation_scope_quota(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        kind: OrganisationQuotaKind,
        quota: Option<Quota>,
    ) -> Result<OrganisationScopeQuota, CoreError> {
        self.policy
            .require(identity.clone(), PlatformRight::ActOnTenant)
            .await?;
        let organisation = self.organisation(organisation_id).await?;

        let before = self
            .quotas
            .organisation_overrides(organisation_id)
            .await?
            .get(&kind)
            .copied();
        let now = Utc::now();

        if before != quota {
            self.quotas
                .set_organisation_override(
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

        self.view(&organisation, kind).await
    }
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

    use chrono::DateTime;

    use super::*;
    use crate::{
        audit::ports::MockAuditRepository,
        organisation::{
            Organisation,
            invitation::{Invitation, InvitationId, InvitationTokenHash, InvitedEmail},
            member::{Member, MemberId},
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

    type ScopeService = OrganisationScopeQuotaServiceImpl<
        InMemoryQuotas,
        Holding,
        MockOrganisationRepository,
        MockAuditRepository,
        NoUsers,
        crate::deployments::ports::MockDeploymentRepository,
        Invitations,
    >;

    struct Invitations(Vec<Invitation>);

    impl InvitationRepository for Invitations {
        async fn list_invitations(&self, _: &OrganisationId) -> Result<Vec<Invitation>, CoreError> {
            Ok(self.0.clone())
        }

        async fn save_invitation(
            &self,
            _: &Invitation,
            _: &InvitationTokenHash,
        ) -> Result<(), CoreError> {
            unreachable!("counting writes nothing")
        }

        async fn find_invitation_by_hash(
            &self,
            _: &InvitationTokenHash,
        ) -> Result<Option<Invitation>, CoreError> {
            unreachable!("counting looks nothing up")
        }

        async fn find_invitation(
            &self,
            _: &OrganisationId,
            _: &InvitationId,
        ) -> Result<Option<Invitation>, CoreError> {
            unreachable!("counting looks nothing up")
        }

        async fn mark_invitation_accepted(
            &self,
            _: &InvitationId,
            _: DateTime<Utc>,
        ) -> Result<(), CoreError> {
            unreachable!("counting writes nothing")
        }

        async fn mark_invitation_revoked(
            &self,
            _: &InvitationId,
            _: DateTime<Utc>,
        ) -> Result<(), CoreError> {
            unreachable!("counting writes nothing")
        }
    }

    fn invitation(now: DateTime<Utc>) -> Invitation {
        Invitation::issue(
            org(),
            InvitedEmail::parse("pilot@acme.test").expect("an address"),
            Vec::new(),
            None,
            now,
        )
        .0
    }

    fn member() -> Member {
        Member {
            id: MemberId(Uuid::from_u128(7)),
            organisation_id: org(),
            user_id: crate::user::UserId(Uuid::from_u128(2)),
            email: "owner@acme.test".to_string(),
            name: "owner".to_string(),
            roles: Vec::new(),
            joined_at: Utc::now(),
            invited_by: None,
        }
    }

    fn free_organisation() -> Organisation {
        Organisation {
            plan: Plan::Free,
            limits: OrganisationLimits::from_plan(&Plan::Free),
            ..organisation()
        }
    }

    fn scope_service(
        quotas: InMemoryQuotas,
        holding: &[PlatformRight],
        invitations: Vec<Invitation>,
    ) -> (ScopeService, Recorded) {
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));

        let mut organisations = MockOrganisationRepository::new();
        organisations.expect_find_by_id().returning(|_| {
            let found = Some(free_organisation());
            Box::pin(async move { Ok(found) })
        });
        organisations
            .expect_list_members()
            .returning(|_| Box::pin(async { Ok(vec![member()]) }));

        let mut deployments = crate::deployments::ports::MockDeploymentRepository::new();
        deployments
            .expect_list_by_organisation()
            .returning(|_| Box::pin(async { Ok(Vec::new()) }));

        let mut audit = MockAuditRepository::new();
        let heard = Arc::clone(&recorded);
        audit.expect_append().returning(move |entry| {
            heard.lock().unwrap().push(entry);
            Box::pin(async { Ok(()) })
        });

        (
            OrganisationScopeQuotaServiceImpl::new(
                quotas,
                Holding(PlatformRights::of(holding.iter().copied())),
                organisations,
                audit,
                NoUsers,
                deployments,
                Invitations(invitations),
            ),
            recorded,
        )
    }

    #[tokio::test]
    async fn a_free_organisation_refuses_a_second_deployment() {
        let refused = require_organisation_room(
            &InMemoryQuotas::default(),
            org(),
            Plan::Free,
            OrganisationQuotaKind::Deployments,
            1,
        )
        .await
        .expect_err("the free plan holds one");

        let CoreError::OrganisationLimitReached {
            limit_type,
            max,
            current,
        } = refused
        else {
            panic!("the refusal was not a limit: {refused}");
        };
        assert_eq!((limit_type.as_str(), max, current), ("deployments", 1, 1));
    }

    #[tokio::test]
    async fn an_organisation_override_allows_the_second_and_refuses_the_third() {
        let quotas = InMemoryQuotas::default().with_organisation_kind(
            org(),
            OrganisationQuotaKind::Deployments,
            Quota::Limited(2),
        );

        require_organisation_room(
            &quotas,
            org(),
            Plan::Free,
            OrganisationQuotaKind::Deployments,
            1,
        )
        .await
        .expect("the second fits");
        assert!(
            require_organisation_room(
                &quotas,
                org(),
                Plan::Free,
                OrganisationQuotaKind::Deployments,
                2
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn the_scope_listing_needs_view_estate() {
        let (service, _) = scope_service(
            InMemoryQuotas::default(),
            &[PlatformRight::OperateFleet],
            Vec::new(),
        );

        assert!(matches!(
            service
                .list_organisation_scope_quotas(caller(), org())
                .await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
    }

    #[tokio::test]
    async fn members_held_counts_members_and_only_pending_invitations() {
        let now = Utc::now();
        let mut revoked = invitation(now);
        revoked.revoked_at = Some(now);
        let mut accepted = invitation(now);
        accepted.accepted_at = Some(now);
        let expired = invitation(now - chrono::Duration::days(30));
        let (service, _) = scope_service(
            InMemoryQuotas::default(),
            &[PlatformRight::ViewEstate],
            vec![invitation(now), revoked, accepted, expired],
        );

        let listed = service
            .list_organisation_scope_quotas(caller(), org())
            .await
            .unwrap();

        let members = listed
            .iter()
            .find(|entry| entry.kind == OrganisationQuotaKind::Members)
            .unwrap();
        assert_eq!(members.held, 2);
        assert_eq!(members.plan_default, Quota::Limited(2));
        assert_eq!(members.effective, Quota::Limited(2));
        let deployments = listed
            .iter()
            .find(|entry| entry.kind == OrganisationQuotaKind::Deployments)
            .unwrap();
        assert_eq!(deployments.held, 0);
    }

    #[tokio::test]
    async fn setting_a_scope_quota_needs_act_on_tenant_and_is_audited() {
        let (denied, recorded) = scope_service(
            InMemoryQuotas::default(),
            &[PlatformRight::ViewEstate],
            Vec::new(),
        );
        assert!(matches!(
            denied
                .set_organisation_scope_quota(
                    caller(),
                    org(),
                    OrganisationQuotaKind::Deployments,
                    Some(Quota::Limited(2))
                )
                .await,
            Err(CoreError::MissingPlatformRight { .. })
        ));
        assert!(recorded.lock().unwrap().is_empty());

        let (service, recorded) = scope_service(
            InMemoryQuotas::default(),
            &[PlatformRight::ActOnTenant],
            Vec::new(),
        );
        let view = service
            .set_organisation_scope_quota(
                caller(),
                org(),
                OrganisationQuotaKind::Deployments,
                Some(Quota::Limited(2)),
            )
            .await
            .unwrap();

        assert_eq!(view.organisation_override, Some(Quota::Limited(2)));
        assert_eq!(view.effective, Quota::Limited(2));
        let entries = recorded.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action.0, "organisation.quota.changed");
    }

    #[tokio::test]
    async fn clearing_a_scope_quota_returns_to_the_plan_and_setting_the_same_writes_nothing() {
        let quotas = InMemoryQuotas::default().with_organisation_kind(
            org(),
            OrganisationQuotaKind::Members,
            Quota::Limited(9),
        );
        let (service, recorded) =
            scope_service(quotas.clone(), &[PlatformRight::ActOnTenant], Vec::new());

        service
            .set_organisation_scope_quota(
                caller(),
                org(),
                OrganisationQuotaKind::Members,
                Some(Quota::Limited(9)),
            )
            .await
            .unwrap();
        assert!(recorded.lock().unwrap().is_empty());

        let view = service
            .set_organisation_scope_quota(caller(), org(), OrganisationQuotaKind::Members, None)
            .await
            .unwrap();

        assert_eq!(view.organisation_override, None);
        assert_eq!(view.effective, Quota::Limited(2));
        assert!(
            quotas
                .organisation_overrides(org())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(recorded.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_unknown_organisation_is_not_found() {
        let mut organisations = MockOrganisationRepository::new();
        organisations
            .expect_find_by_id()
            .returning(|_| Box::pin(async { Ok(None) }));
        let service = OrganisationScopeQuotaServiceImpl::new(
            InMemoryQuotas::default(),
            Holding(PlatformRights::of([PlatformRight::ViewEstate])),
            organisations,
            MockAuditRepository::new(),
            NoUsers,
            crate::deployments::ports::MockDeploymentRepository::new(),
            Invitations(Vec::new()),
        );

        assert!(matches!(
            service
                .list_organisation_scope_quotas(caller(), org())
                .await,
            Err(CoreError::OrganisationNotFound { .. })
        ));
    }
}
