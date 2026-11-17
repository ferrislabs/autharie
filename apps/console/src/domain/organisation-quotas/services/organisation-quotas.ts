export const organisationQuotasQuery = (organisationId: string) =>
  window.api.get('/platform/organisations/{organisation_id}/organisation-quotas', {
    path: { organisation_id: organisationId },
  })

export const setOrganisationQuotaMutation = () =>
  window.api.mutation('put', '/platform/organisations/{organisation_id}/organisation-quotas/{kind}')
