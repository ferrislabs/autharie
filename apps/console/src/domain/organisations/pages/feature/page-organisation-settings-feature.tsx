import { useGetOrganisation, useUpdateOrganisation } from '@/api/organisation.api'
import { RequiresPermission } from '@/components/layout/requires-permission'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { CAN } from '@/domain/organisations/permissions'
import { PageOrganisationSettings } from '../ui/page-organisation-settings'

export default function PageOrganisationSettingsFeature() {
  const organisationId = useResolvedOrganisationId() ?? null
  const { can } = useMyPermissions()

  const organisation = useGetOrganisation(organisationId)
  const update = useUpdateOrganisation()

  return (
    <RequiresPermission permission={CAN.viewOrganisation} what='see the settings'>
      <PageOrganisationSettings
        key={organisationId}
        mcpEnabled={organisation.data?.data.mcp_enabled ?? false}
        canManage={can(CAN.manageOrganisation)}
        isLoading={organisation.isLoading}
        isSaving={update.isPending}
        refusal={update.error?.message ?? organisation.error?.message}
        onMcpEnabledChange={(enabled) => {
          if (!organisationId || update.isPending) return

          update.mutate({
            path: { organisation_id: organisationId },
            body: { mcp_enabled: enabled },
          })
        }}
      />
    </RequiresPermission>
  )
}
