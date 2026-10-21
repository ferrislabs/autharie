import { useNavigate } from '@tanstack/react-router'
import PageCreateDeployment from '../ui/page-create-deployment'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { useCreateDeployment } from '@/api/deployment.api'
import { useGetBillingAccount, useGetTiers } from '@/api/billing.api'
import { accountOf } from '@/domain/billing/billing'
import { useGetOffers } from '@/api/offer.api'
import { useGetPublishedReleases } from '@/api/release.api'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import {
  toCreateDeploymentRequest,
  type CreateDeploymentForm,
} from '../../create-deployment-request'

export default function PageCreateDeploymentFeature() {
  const navigate = useNavigate()
  const organisationPath = useOrganisationPath()
  const organisationId = useResolvedOrganisationId()
  const createDeployment = useCreateDeployment()
  const offers = useGetOffers(organisationId ?? null)
  const ferriskey = useGetPublishedReleases('ferriskey')
  const { can } = useMyPermissions()
  const tiers = useGetTiers(organisationId ?? null)
  const canViewBilling = can(CAN.viewBilling)
  const account = useGetBillingAccount(organisationId ?? null, canViewBilling)

  const handleCreate = (form: CreateDeploymentForm) => {
    if (!organisationId) return

    createDeployment.mutate(
      {
        path: { organisation_id: organisationId },
        body: toCreateDeploymentRequest(form),
      },
      { onSuccess: () => navigate({ to: organisationPath('/deployments') }) },
    )
  }

  return (
    <PageCreateDeployment
      onSubmit={handleCreate}
      isSubmitting={createDeployment.isPending}
      refusal={
        createDeployment.error instanceof Error ? createDeployment.error.message : undefined
      }
      offers={offers.data?.data ?? []}
      offersLoading={offers.isLoading}
      releases={ferriskey.data?.data ?? []}
      releasesLoading={ferriskey.isLoading}
      tiers={tiers.data?.data ?? []}
      tiersLoading={tiers.isLoading}
      billing={{
        canManage: can(CAN.manageBilling),
        account: accountOf(canViewBilling, account),
      }}
    />
  )
}
