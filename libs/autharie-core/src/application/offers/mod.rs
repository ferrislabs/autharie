use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    offers::{
        Offer, OrganisationOffer, ports::OrganisationOfferService,
        service::OrganisationOfferServiceImpl,
    },
    organisation::OrganisationId,
};
use autharie_macros::transactional;

use crate::{AutharieService, policy::PlatformRightsPolicy};

impl OrganisationOfferService for AutharieService {
    #[transactional(offer_override, platform_operator, organisation, audit, user)]
    async fn list_organisation_offers(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
    ) -> Result<Vec<OrganisationOffer>, CoreError> {
        OrganisationOfferServiceImpl::new(
            offer_override_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
        )
        .list_organisation_offers(identity, organisation_id)
        .await
    }

    #[transactional(offer_override, platform_operator, organisation, audit, user)]
    async fn set_organisation_offer(
        &self,
        identity: Identity,
        organisation_id: OrganisationId,
        offer: Offer,
        open: Option<bool>,
    ) -> Result<OrganisationOffer, CoreError> {
        OrganisationOfferServiceImpl::new(
            offer_override_repository,
            PlatformRightsPolicy::new(platform_operator_repository),
            organisation_repository,
            audit_repository,
            user_repository,
        )
        .set_organisation_offer(identity, organisation_id, offer, open)
        .await
    }
}
