export const offerOverridesQuery = (organisationId: string) =>
  window.api.get('/platform/organisations/{organisation_id}/offers', {
    path: { organisation_id: organisationId },
  })

export const setOfferOverrideMutation = () =>
  window.api.mutation('put', '/platform/organisations/{organisation_id}/offers/{offer}')
