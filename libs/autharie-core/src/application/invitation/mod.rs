use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    organisation::{
        OrganisationId,
        invitation::{Invitation, InvitationId, InvitationToken, InvitedEmail},
        invitation_service::InvitationServiceImpl,
        member::Member,
        ports::InvitationService,
    },
    role::RoleId,
};
use autharie_macros::transactional;
use autharie_postgres::organisation::PostgresOrganisationRepository;

use crate::{AutharieService, infrastructure::role::permissions_in, policy::AuthariePolicy};

impl InvitationService for AutharieService {
    #[transactional(organisation, role, user)]
    async fn invite(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        email: InvitedEmail,
        roles: Vec<RoleId>,
    ) -> Result<(Invitation, InvitationToken), CoreError> {
        InvitationServiceImpl::new(
            organisation_repository,
            PostgresOrganisationRepository::new(&tx),
            role_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .invite(identity, organisation_id, email, roles)
        .await
    }

    #[transactional(organisation, role, user)]
    async fn list_invitations(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<Invitation>, CoreError> {
        InvitationServiceImpl::new(
            organisation_repository,
            PostgresOrganisationRepository::new(&tx),
            role_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .list_invitations(identity, organisation_id)
        .await
    }

    #[transactional(organisation, role, user)]
    async fn revoke_invitation(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        invitation_id: InvitationId,
    ) -> Result<Invitation, CoreError> {
        InvitationServiceImpl::new(
            organisation_repository,
            PostgresOrganisationRepository::new(&tx),
            role_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .revoke_invitation(identity, organisation_id, invitation_id)
        .await
    }

    /// In one transaction, because it writes three things: the membership,
    /// its roles, and the stamp that closes the invitation. Two of those
    /// landing without the third is either a member who holds nothing or a
    /// link that still works after it was used.
    #[transactional(organisation, role, user)]
    async fn accept_invitation(
        &self,
        identity: Identity,
        token: InvitationToken,
    ) -> Result<Member, CoreError> {
        InvitationServiceImpl::new(
            organisation_repository,
            PostgresOrganisationRepository::new(&tx),
            role_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
        )
        .accept_invitation(identity, token)
        .await
    }
}
