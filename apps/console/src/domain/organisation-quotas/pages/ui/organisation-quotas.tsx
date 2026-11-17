import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { UNLIMITED, describeQuota, parseLimit, sourceLabel } from '@/domain/quotas/quota'
import type { Quota } from '@/domain/quotas/quota'
import { heldLabel, isOverLimit } from '../../organisation-quota'
import type { OrganisationQuotaKind, OrganisationQuotaView } from '../../types'

interface Props {
  quotas: OrganisationQuotaView[]
  isLoading: boolean
  canChange: boolean
  pending?: OrganisationQuotaKind
  refusal?: string
  onChange: (kind: OrganisationQuotaKind, quota: Quota | null) => void
}

function QuotaRow({
  quota,
  canChange,
  pending,
  onChange,
}: {
  quota: OrganisationQuotaView
  canChange: boolean
  pending: boolean
  onChange: (kind: OrganisationQuotaKind, quota: Quota | null) => void
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
        <p className='mt-1 text-xs text-muted-foreground'>
          Plan default: {describeQuota(quota.plan_default)} · {sourceLabel(override)}
          {override && (
            <>
              {': '}
              <span className='font-medium text-foreground'>{describeQuota(override)}</span>
            </>
          )}
        </p>
        <p className='mt-1 text-xs text-muted-foreground'>
          Effective{' '}
          <span className='font-medium text-foreground'>{describeQuota(quota.effective)}</span>
          {' · '}
          Held{' '}
          <span
            className={
              isOverLimit(quota) ? 'font-medium text-destructive' : 'font-medium text-foreground'
            }
          >
            {heldLabel(quota)}
          </span>
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

export function OrganisationQuotas({
  quotas,
  isLoading,
  canChange,
  pending,
  refusal,
  onChange,
}: Props) {
  return (
    <section className='mt-10'>
      <h2 className='text-lg font-semibold'>Organisation quotas</h2>
      <p className='mt-1 text-sm text-muted-foreground'>
        What the organisation as a whole may hold. An override replaces the plan&apos;s default.
        Lowering one deletes nothing and refuses only the next create.
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
