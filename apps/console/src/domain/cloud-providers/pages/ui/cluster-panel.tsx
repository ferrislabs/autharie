import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { OptionCard } from '@/domain/deployments/pages/ui/components/option-card'
import type { Schemas } from '@/api/api.client'
import { formatEur } from '../../money'
import { MODES, MODE_RULES, controlPlaneAllowed, locksNodes, type ProfileDraft } from '../../profile'
import { describeProfile, replicasBelowInUse } from '../../resize'
import type { ClusterMode, ProviderOffers } from '../../types/cloud-provider'
import { Field, type EstimateView } from './customer-cloud-form'

interface Props {
  profile: Schemas.ClusterProfile
  regions: string[]
  region: string
  onRegion: (region: string) => void
  offers?: ProviderOffers
  offersLoading: boolean
  offersFailed?: string
  draft: ProfileDraft
  onMode: (mode: ClusterMode) => void
  onControlPlane: (id: string) => void
  onMinNodes: (value: number) => void
  onMaxNodes: (value: number) => void
  estimate: EstimateView
  canApply: boolean
  applying: boolean
  onApply: () => void
  failure?: string
  applied?: Schemas.ClusterProfile
}

export function ClusterPanel({
  profile,
  regions,
  region,
  onRegion,
  offers,
  offersLoading,
  offersFailed,
  draft,
  onMode,
  onControlPlane,
  onMinNodes,
  onMaxNodes,
  estimate,
  canApply,
  applying,
  onApply,
  failure,
  applied,
}: Props) {
  const shrinking = replicasBelowInUse(draft, profile)

  return (
    <section aria-labelledby='cluster-panel-title' className='space-y-4 rounded-lg border p-4'>
      <div>
        <h2 id='cluster-panel-title' className='text-base font-semibold'>
          Cluster
        </h2>
        <p className='text-sm text-muted-foreground'>Now: {describeProfile(profile)}</p>
      </div>

      <Field label='Region (for prices)' htmlFor='cluster-region'>
        <Select value={region} onValueChange={onRegion}>
          <SelectTrigger id='cluster-region' className='w-full sm:w-64'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {regions.map((entry) => (
              <SelectItem key={entry} value={entry}>
                {entry}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      {offersLoading && <Skeleton className='h-32 w-full' />}

      {offersFailed && (
        <p role='alert' className='text-sm text-destructive'>
          {offersFailed}
        </p>
      )}

      {offers && (
        <>
          <div className='grid gap-3 sm:grid-cols-3'>
            {MODES.map((mode) => (
              <OptionCard
                key={mode}
                selected={draft.mode === mode}
                onSelect={() => onMode(mode)}
                label={MODE_RULES[mode].label}
                description={MODE_RULES[mode].description}
              />
            ))}
          </div>

          <div className='grid gap-4 sm:grid-cols-2'>
            <Field label='Control plane' htmlFor='cluster-control-plane'>
              <Select value={draft.controlPlaneId} onValueChange={onControlPlane}>
                <SelectTrigger id='cluster-control-plane' className='w-full'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {offers.control_planes.map((offer) => (
                    <SelectItem
                      key={offer.id}
                      value={offer.id}
                      disabled={!controlPlaneAllowed(draft.mode, offer)}
                    >
                      {offer.kind === 'mutualized' ? 'Mutualized' : `Dedicated ${offer.id}`} ·{' '}
                      {formatEur(offer.monthly_price)}/month
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>

            <Field label='Node type' htmlFor='cluster-node-type'>
              <Input id='cluster-node-type' value={profile.node_type} readOnly disabled />
              <p className='text-xs text-muted-foreground'>
                The node type of a running cluster cannot change.
              </p>
            </Field>

            <Field label='Minimum nodes' htmlFor='cluster-min-nodes'>
              <Input
                id='cluster-min-nodes'
                type='number'
                min={MODE_RULES[draft.mode].minNodes}
                value={draft.minNodes}
                disabled={locksNodes(draft.mode)}
                onChange={(event) => onMinNodes(Number(event.target.value))}
              />
            </Field>

            <Field label='Maximum nodes' htmlFor='cluster-max-nodes'>
              <Input
                id='cluster-max-nodes'
                type='number'
                min={draft.minNodes}
                value={draft.maxNodes}
                disabled={locksNodes(draft.mode)}
                onChange={(event) => onMaxNodes(Number(event.target.value))}
              />
            </Field>
          </div>

          <p className='text-xs text-muted-foreground'>Replicas: {draft.replication}, set by the mode.</p>

          {shrinking && (
            <p role='alert' className='text-sm text-destructive'>
              {shrinking}
            </p>
          )}

          <div aria-live='polite' className='text-sm'>
            {estimate.refusal ? (
              <p role='alert' className='text-destructive'>
                {estimate.refusal}
              </p>
            ) : estimate.text ? (
              <p>
                <span className='font-medium'>Estimated cost:</span> {estimate.text}.
              </p>
            ) : estimate.loading ? (
              <p className='text-muted-foreground'>Estimating the cost…</p>
            ) : null}
          </div>
        </>
      )}

      {failure && (
        <p role='alert' className='text-sm text-destructive'>
          {failure}
        </p>
      )}

      {applied && !failure && (
        <p role='status' className='text-sm text-green-700 dark:text-green-400'>
          Resized. The cluster now runs {describeProfile(applied)}.
        </p>
      )}

      <Button type='button' disabled={!canApply || applying} onClick={onApply}>
        {applying ? 'Applying…' : 'Apply'}
      </Button>
    </section>
  )
}
