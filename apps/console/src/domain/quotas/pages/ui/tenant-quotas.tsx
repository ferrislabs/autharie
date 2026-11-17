import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import type { Schemas } from '@/api/api.client'
import { UNLIMITED, describeQuota, parseLimit, sourceLabel } from '../../quota'
import type { Quota, QuotaKind } from '../../quota'

type TenantQuota = Schemas.TenantQuotaView

interface Props {
  quotas: TenantQuota[]
  isLoading: boolean
  canChange: boolean
  pending?: QuotaKind
  refusal?: string
  /** `null` clears the override and returns to the offer default. */
  onChange: (kind: QuotaKind, quota: Quota | null) => void
}

function QuotaRow({
  quota,
  canChange,
  pending,
  onChange,
}: {
  quota: TenantQuota
  canChange: boolean
  pending: boolean
  onChange: (kind: QuotaKind, quota: Quota | null) => void
}) {
  const [draft, setDraft] = useState('')
  const [problem, setProblem] = useState<string>()
  const override = quota.override ?? null

  const save = () => {
    const parsed = parseLimit(draft)
    if (!parsed.ok) {
      setProblem(parsed.reason)
      return
    }

    setProblem(undefined)
    setDraft('')
    onChange(quota.kind, parsed.quota)
  }

  return (
    <div className='flex flex-wrap items-start justify-between gap-4 px-4 py-3'>
      <div className='min-w-0'>
        <p className='font-mono text-sm font-medium'>{quota.kind}</p>
        <p className='text-xs text-muted-foreground'>{quota.description}</p>
        {!quota.enforced && (
          <p className='mt-1 text-xs text-muted-foreground'>Measured, never enforced.</p>
        )}
        <p className='mt-1 text-xs text-muted-foreground'>
          Default by offer:{' '}
          {quota.defaults
            .map((entry) => `${entry.offer} ${describeQuota(entry.quota)}`)
            .join(' · ')}
        </p>
        <p className='mt-1 text-xs text-muted-foreground'>
          {sourceLabel(override)}
          {override && (
            <>
              {': '}
              <span className='font-medium text-foreground'>{describeQuota(override)}</span> for
              every deployment of this organisation
            </>
          )}
        </p>
      </div>

      <div className='flex flex-col items-end gap-1'>
        <div className='flex flex-wrap items-center justify-end gap-1' role='group'>
          <Input
            className='h-8 w-24'
            inputMode='numeric'
            aria-label={`Override for ${quota.kind}`}
            placeholder='Limit'
            value={draft}
            disabled={!canChange || pending}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => event.key === 'Enter' && save()}
          />
          <Button size='sm' disabled={!canChange || pending || draft === ''} onClick={save}>
            Set
          </Button>
          <Button
            size='sm'
            variant='outline'
            disabled={!canChange || pending || override?.type === 'unlimited'}
            onClick={() => onChange(quota.kind, UNLIMITED)}
          >
            Unlimited
          </Button>
          <Button
            size='sm'
            variant='outline'
            disabled={!canChange || pending || !override}
            onClick={() => onChange(quota.kind, null)}
          >
            Reset to default
          </Button>
        </div>
        {problem && (
          <p className='text-xs text-destructive' role='alert'>
            {problem}
          </p>
        )}
      </div>
    </div>
  )
}

export function TenantQuotas({ quotas, isLoading, canChange, pending, refusal, onChange }: Props) {
  return (
    <section className='mt-10'>
      <h2 className='text-lg font-semibold'>Quotas</h2>
      <p className='mt-1 text-sm text-muted-foreground'>
        What each deployment may hold. An override replaces the offer&apos;s default for every
        deployment of this organisation, whatever its plan. Lowering one deletes nothing and refuses
        only the next create.
      </p>

      {refusal && (
        <div
          className='mt-4 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm'
          role='alert'
        >
          {refusal}
        </div>
      )}

      <div className='mt-4 divide-y rounded-lg border'>
        {isLoading ? (
          <Skeleton className='h-16' />
        ) : (
          quotas.map((quota) => (
            <QuotaRow
              key={quota.kind}
              quota={quota}
              canChange={canChange}
              pending={pending === quota.kind}
              onChange={onChange}
            />
          ))
        )}
      </div>
    </section>
  )
}
