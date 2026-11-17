export const tenantQuotasQuery = (organisationId: string) =>
  window.api.get('/platform/organisations/{organisation_id}/quotas', {
    path: { organisation_id: organisationId },
  })

export const setTenantQuotaMutation = () =>
  window.api.mutation('put', '/platform/organisations/{organisation_id}/quotas/{kind}')

export const deploymentQuotasQuery = (organisationId: string, deploymentId: string) =>
  window.api.get('/organisations/{organisation_id}/deployments/{deployment_id}/quotas', {
    path: { organisation_id: organisationId, deployment_id: deploymentId },
  })
