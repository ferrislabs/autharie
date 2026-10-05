import type { ReactNode } from 'react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { OptionCard } from '@/domain/deployments/pages/ui/components/option-card'
import { formatEur } from '../../money'
import {
  MODES,
  MODE_RULES,
  controlPlaneAllowed,
  locksNodes,
  type ProfileDraft,
} from '../../profile'
import type { CloudCredential, ClusterMode, ProviderOffers } from '../../types/cloud-provider'

export interface EstimateView {
  loading: boolean
  text?: string
  refusal?: string
}

interface Props {
  credentials: CloudCredential[]
  credentialId: string
  onCredential: (id: string) => void
  regions: string[]
  region: string
  onRegion: (region: string) => void
  offers?: ProviderOffers
  offersLoading: boolean
  offersFailed?: string
  draft: ProfileDraft | null
  onMode: (mode: ClusterMode) => void
  onControlPlane: (id: string) => void
  onNodeType: (nodeType: string) => void
  onMinNodes: (value: number) => void
  onMaxNodes: (value: number) => void
  estimate: EstimateView
}

export function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className='space-y-2'>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  )
}

export function CustomerCloudForm({
  credentials,
  credentialId,
  onCredential,
  regions,
  region,
  onRegion,
  offers,
  offersLoading,
  offersFailed,
  draft,
  onMode,
  onControlPlane,
  onNodeType,
  onMinNodes,
  onMaxNodes,
  estimate,
}: Props) {
  return (
    <div className='space-y-6 rounded-lg border p-4'>
      <div className='grid gap-4 sm:grid-cols-2'>
        <Field label='Cloud account' htmlFor='cloud-account'>
          <Select value={credentialId} onValueChange={onCredential}>
            <SelectTrigger id='cloud-account' className='w-full'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {credentials.map((credential) => (
                <SelectItem key={credential.id} value={credential.id}>
                  {credential.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>

        <Field label='Region' htmlFor='cloud-region'>
          <Select value={region} onValueChange={onRegion}>
            <SelectTrigger id='cloud-region' className='w-full'>
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
      </div>

      {offersLoading && <Skeleton className='h-32 w-full' />}

      {offersFailed && (
        <p role='alert' className='text-sm text-destructive'>
          {offersFailed}
        </p>
      )}

      {offers && draft && (
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
            <Field label='Control plane' htmlFor='control-plane'>
              <Select value={draft.controlPlaneId} onValueChange={onControlPlane}>
                <SelectTrigger id='control-plane' className='w-full'>
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
              {!MODE_RULES[draft.mode].dedicatedControlPlane && (
                <p className='text-xs text-muted-foreground'>
                  A dev cluster uses the mutualized control plane only.
                </p>
              )}
            </Field>

            <Field label='Node type' htmlFor='node-type'>
              <Select value={draft.nodeType} onValueChange={onNodeType}>
                <SelectTrigger id='node-type' className='w-full'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {offers.node_types.map((offer) => (
                    <SelectItem key={offer.node_type} value={offer.node_type}>
                      {offer.node_type} · {formatEur(offer.monthly_price)}/month
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>

            <Field label='Minimum nodes' htmlFor='min-nodes'>
              <Input
                id='min-nodes'
                type='number'
                min={MODE_RULES[draft.mode].minNodes}
                value={draft.minNodes}
                disabled={locksNodes(draft.mode)}
                onChange={(event) => onMinNodes(Number(event.target.value))}
              />
            </Field>

            <Field label='Maximum nodes' htmlFor='max-nodes'>
              <Input
                id='max-nodes'
                type='number'
                min={draft.minNodes}
                value={draft.maxNodes}
                disabled={locksNodes(draft.mode)}
                onChange={(event) => onMaxNodes(Number(event.target.value))}
              />
            </Field>
          </div>

          <p className='text-xs text-muted-foreground'>
            Replicas: {draft.replication}, set by the mode.
          </p>

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
    </div>
  )
}
