import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { cn } from '@/lib/utils'
import { DeploymentKindLabel } from '../pages/ui/components/deployment-kind'
import type { DeploymentKind } from '../types/deployment'

export interface InstanceFacts {
  kind: DeploymentKind
  endpoint?: string
  issuer?: string
  version: string
  latestVersion?: string
  offer: string
  resources: string
  runsOn: string
  region?: string
  createdAt: number
}

function CopyValue({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false)

  return (
    <div className='group flex min-w-0 items-center gap-1'>
      <span className='truncate font-mono text-xs' title={value}>
        {value.replace(/^https?:\/\//, '')}
      </span>
      <button
        type='button'
        aria-label={`Copy ${label}`}
        onClick={() => {
          void navigator.clipboard.writeText(value)
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1500)
        }}
        className='shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground'
      >
        {copied ? <Check className='h-3.5 w-3.5 text-[#0ca30c]' /> : <Copy className='h-3.5 w-3.5' />}
      </button>
    </div>
  )
}

function Row({ term, children }: { term: string; children: React.ReactNode }) {
  return (
    <div className='grid grid-cols-[6.5rem_1fr] items-center gap-3 py-2'>
      <dt className='text-xs text-muted-foreground'>{term}</dt>
      <dd className='min-w-0 text-sm'>{children}</dd>
    </div>
  )
}

const created = new Intl.DateTimeFormat('en', { dateStyle: 'medium' })

export function InstanceSummary({ facts }: { facts: InstanceFacts }) {
  const outdated = facts.latestVersion !== undefined && facts.latestVersion !== facts.version

  return (
    <section className='rounded-lg border bg-card' aria-label='Instance'>
      <div className='border-b px-4 py-3'>
        <h2 className='text-sm font-semibold'>Instance</h2>
      </div>
      <dl className='divide-y px-4 py-1'>
        <Row term='Provider'>
          <DeploymentKindLabel kind={facts.kind} />
        </Row>
        {facts.endpoint && (
          <Row term='Endpoint'>
            <CopyValue value={facts.endpoint} label='endpoint' />
          </Row>
        )}
        {facts.issuer && (
          <Row term='Issuer'>
            <CopyValue value={facts.issuer} label='issuer URL' />
          </Row>
        )}
        <Row term='Version'>
          <span className='flex items-center gap-2'>
            <span className='font-mono text-xs'>{facts.version}</span>
            <span
              className={cn(
                'text-xs',
                outdated ? 'text-primary' : 'text-muted-foreground',
              )}
            >
              {outdated ? `${facts.latestVersion} available` : 'Latest'}
            </span>
          </span>
        </Row>
        <Row term='Offer'>
          <span className='flex flex-col'>
            <span>{facts.offer}</span>
            <span className='text-xs text-muted-foreground'>{facts.resources}</span>
          </span>
        </Row>
        <Row term='Runs on'>
          <span className='flex flex-col'>
            <span>{facts.runsOn}</span>
            {facts.region && <span className='text-xs text-muted-foreground'>{facts.region}</span>}
          </span>
        </Row>
        <Row term='Created'>{created.format(facts.createdAt)}</Row>
      </dl>
    </section>
  )
}
