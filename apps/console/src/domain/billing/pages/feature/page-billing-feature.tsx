import { useGetDeployments } from '@/api/deployment.api'
import {
  useCreatePaymentMethodSession,
  useGetBillingAccount,
  useGetStatement,
  useGetStatements,
  usePutBillingAccount,
} from '@/api/billing.api'
import { RequiresPermission } from '@/components/layout/requires-permission'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { CAN } from '@/domain/organisations/permissions'
import { PageBilling } from '../ui/page-billing'
import { estimateFailure, isFirstLoad } from './billing-load'
import { accountRefusal, loadFailure, paymentMethodRefusal } from './billing-refusals'

export default function PageBillingFeature() {
  const organisationId = useResolvedOrganisationId() ?? null
  const { can } = useMyPermissions()
  const canView = can(CAN.viewBilling)

  const account = useGetBillingAccount(organisationId, canView)
  const statements = useGetStatements(organisationId, canView)
  const period = statements.data?.current_period ?? null
  const estimate = useGetStatement(organisationId, period, canView)
  const deployments = useGetDeployments()

  const save = usePutBillingAccount()
  const session = useCreatePaymentMethodSession()

  const names = new Map(
    (deployments.data?.data ?? []).map((deployment) => [deployment.id, deployment.name] as const),
  )

  return (
    <RequiresPermission permission={CAN.viewBilling} what='see the billing'>
      <PageBilling
        key={organisationId}
        account={account.data ?? null}
        accountFailure={loadFailure(account.error)}
        statements={statements.data?.data ?? []}
        statementsFailure={loadFailure(statements.error)}
        estimate={estimate.data}
        estimateFailure={estimateFailure(period === null ? statements.error : null, estimate.error)}
        names={names}
        canManage={can(CAN.manageBilling)}
        isLoading={isFirstLoad([account, statements, deployments])}
        estimateLoading={estimate.isLoading || (period === null && statements.isFetching)}
        isSaving={save.isPending}
        isOpening={session.isPending}
        saveRefusal={accountRefusal(save.error)}
        paymentMethodRefusal={paymentMethodRefusal(session.error)}
        onSave={(request) => {
          if (!organisationId || save.isPending) return

          save.mutate({ path: { organisation_id: organisationId }, body: request })
        }}
        onAddPaymentMethod={() => {
          if (!organisationId || session.isPending) return

          session.mutate(
            { path: { organisation_id: organisationId } },
            {
              onSuccess: async (answer) => {
                const opened = await answer.json()
                window.location.assign(opened.url)
              },
            },
          )
        }}
      />
    </RequiresPermission>
  )
}
