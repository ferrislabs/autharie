export const platformFeaturesQuery = () => window.api.get('/platform/features')

export const tenantFeaturesQuery = (organisationId: string) =>
  window.api.get('/platform/organisations/{organisation_id}/features', {
    path: { organisation_id: organisationId },
  })

export const enabledFeaturesQuery = (organisationId: string) =>
  window.api.get('/organisations/{organisation_id}/feature-flags', {
    path: { organisation_id: organisationId },
  })

export const setFeatureDefaultMutation = () =>
  window.api.mutation('put', '/platform/features/{key}')

export const setTenantFeatureMutation = () =>
  window.api.mutation('put', '/platform/organisations/{organisation_id}/features/{key}')
