import type { Schemas } from '@/api/api.client'
import { Page, PageTitle, Section } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import { BillingAccountForm } from './components/billing-account-form'
import { MonthToDate } from './components/month-to-date'
import { PaymentMethodPanel } from './components/payment-method-panel'
import { StatementsTable } from './components/statements-table'

interface Props {
  account: Schemas.BillingAccountResponse | null
  accountFailure?: string
  statements: ReadonlyArray<Schemas.StatementSummary>
  statementsFailure?: string
  estimate?: Schemas.StatementResponse
  estimateFailure?: string
  names: ReadonlyMap<string, string>
  canManage: boolean
  isLoading: boolean
  isSaving: boolean
  isOpening: boolean
  saveRefusal?: string
  paymentMethodRefusal?: string
  onSave: (request: Schemas.BillingAccountRequest) => void
  onAddPaymentMethod: () => void
}

function Failure({ message }: { message: string }) {
  return (
    <p role='alert' className='text-sm text-destructive'>
      {message}
    </p>
  )
}

export function PageBilling({
  account,
  accountFailure,
  statements,
  statementsFailure,
  estimate,
  estimateFailure,
  names,
  canManage,
  isLoading,
  isSaving,
  isOpening,
  saveRefusal,
  paymentMethodRefusal,
  onSave,
  onAddPaymentMethod,
}: Props) {
  if (isLoading) {
    return (
      <Page>
        <Skeleton className='h-64 w-full' />
      </Page>
    )
  }

  return (
    <Page>
      <PageTitle title='Billing' />

      <p className='mt-4 text-sm text-muted-foreground'>
        Who is invoiced, how they pay, and what each month costs. Amounts exclude VAT.
      </p>

      <div className='mt-6 space-y-8'>
        <Section
          title='Billing account'
          aside={
            account && (
              <StatusBadge tone={account.complete ? 'success' : 'warning'}>
                {account.complete ? 'Complete' : 'Incomplete'}
              </StatusBadge>
            )
          }
        >
          <p className='text-sm text-muted-foreground'>
            A paid plan needs a name, an e-mail, an address and a valid payment method.
          </p>
          {accountFailure ? (
            <Failure message={accountFailure} />
          ) : (
            <BillingAccountForm
              account={account}
              canManage={canManage}
              isSaving={isSaving}
              refusal={saveRefusal}
              onSave={onSave}
            />
          )}
        </Section>

        <Section title='Payment method'>
          <PaymentMethodPanel
            status={account?.payment_method}
            hasAccount={account !== null}
            canManage={canManage}
            isOpening={isOpening}
            refusal={paymentMethodRefusal}
            onAdd={onAddPaymentMethod}
          />
        </Section>

        <Section title='This month so far'>
          {estimateFailure ? (
            <Failure message={estimateFailure} />
          ) : (
            <MonthToDate estimate={estimate} names={names} />
          )}
        </Section>

        <Section title='Statements'>
          {statementsFailure ? (
            <Failure message={statementsFailure} />
          ) : (
            <StatementsTable statements={statements} />
          )}
        </Section>
      </div>
    </Page>
  )
}
