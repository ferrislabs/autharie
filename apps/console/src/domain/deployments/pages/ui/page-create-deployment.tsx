import type { Schemas } from '@/api/api.client'
import { Button } from '@/components/ui/button'
import { newestInstallable } from '@/domain/releases/catalogue'
import type { BillingAccess } from '@/domain/billing/billing'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Page, PageTitle, Section } from '@/components/layout/page'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { createBlock } from '../../create-deployment-request'
import { type Environment } from '../../types/deployment'
import { OFFER_COPY, openOffers, type Offer, type OfferAvailability } from '../../offers'
import { FREE_OFFER, FREE_OFFER_COPY, isFreeOnly } from '../../free-offer'
import { firstOpenPlan, isOpen, offerFor, planIsOpen } from '../../pricing/offer-for'
import { priceOf, type PlanId, type Selection } from '../../pricing/model'
import { applyChange, STARTER_MOVED_NOTICE } from '../../pricing/selection'
import { FreeOfferSection } from './components/free-offer-section'
import { PlanPicker } from './components/plan-picker'
import { PriceSummary } from './components/price-summary'
import { TierPicker } from './components/tier-picker'
import { VolumePicker } from './components/volume-picker'

const PLANS: PlanId[] = ['starter', 'business', 'scale']

interface Props {
  onSubmit: (data: {
    name: string
    version: string
    environment: Environment
    offer: Offer
    tier?: string
  }) => void
  isSubmitting?: boolean
  /** What the platform said when the last attempt failed. */
  refusal?: string
  offers: OfferAvailability[]
  offersLoading: boolean
  releases: Schemas.Release[]
  releasesLoading: boolean
  tiers: Schemas.TierResponse[]
  billing: BillingAccess
}

interface CreateActionsProps {
  canSubmit: boolean
  block: string | null
  isSubmitting: boolean
  onCancel: () => void
}

export function CreateActions({ canSubmit, block, isSubmitting, onCancel }: CreateActionsProps) {
  return (
    <div className='space-y-2'>
      {block && (
        <p
          role='status'
          className='rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'
        >
          {block}
        </p>
      )}
      <div className='flex items-center justify-end gap-2'>
        <Button type='button' variant='ghost' onClick={onCancel}>
          Cancel
        </Button>
        <Button type='submit' disabled={!canSubmit || block !== null}>
          {isSubmitting ? 'Creating…' : 'Create deployment'}
        </Button>
      </div>
    </div>
  )
}

export default function PageCreateDeployment({
  onSubmit,
  isSubmitting = false,
  refusal,
  offers,
  offersLoading,
  releases,
  releasesLoading,
  tiers,
  billing,
}: Props) {
  const navigate = useNavigate()
  const organisationPath = useOrganisationPath()

  const [name, setName] = useState('')
  const [environment, setEnvironment] = useState<Environment>('development')
  const [picked, setPlan] = useState<PlanId>('business')
  const [volume, setVolume] = useState(10000)
  const [movedToBusiness, setMovedToBusiness] = useState(false)
  const [pickedTier, setPickedTier] = useState<string | null>(null)

  const open = openOffers(offers)
  const freeOnly = isFreeOnly(open)
  const version = newestInstallable(releases) ?? ''

  const closedPlans = PLANS.filter((id) => !planIsOpen(id, open))
  const plan = closedPlans.includes(picked) ? firstOpenPlan(open) : picked

  const selection: Selection = {
    hosting: 'managed',
    engine: 'ferriskey',
    plan: plan ?? picked,
    volume,
  }
  const price = priceOf(selection)

  const derived = offerFor(selection.plan, volume)
  const derivedClosed = !freeOnly && plan !== undefined && !isOpen(derived, open)
  const chosen: Offer | undefined = freeOnly
    ? FREE_OFFER
    : plan !== undefined && !derivedClosed
      ? derived
      : undefined

  const commit = (patch: Partial<Selection>) => {
    const change = applyChange(selection, patch)
    setPlan(change.selection.plan)
    setVolume(change.selection.volume)
    setMovedToBusiness(change.movedToBusiness)
  }

  const tier = tiers.find((candidate) => candidate.code === pickedTier) ?? tiers[0]
  const block = createBlock(tier, billing)

  const canSubmit = !!chosen && name.trim() !== '' && version !== '' && !isSubmitting

  return (
    <Page className='max-w-3xl'>
      <PageTitle title='New deployment' />

      <form
        className='mt-8 space-y-8'
        onSubmit={(e) => {
          e.preventDefault()
          if (chosen && block === null) {
            onSubmit({
              name,
              version,
              environment,
              offer: chosen,
              tier: tier?.code,
            })
          }
        }}
      >
        <Section title='Details'>
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='space-y-2'>
              <Label htmlFor='name'>Name</Label>
              <Input
                id='name'
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder='auth'
                required
              />
            </div>

            <div className='space-y-2'>
              <Label htmlFor='environment'>Environment</Label>
              <Select
                value={environment}
                onValueChange={(value) => setEnvironment(value as Environment)}
              >
                <SelectTrigger id='environment' className='w-full'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value='development'>Development</SelectItem>
                  <SelectItem value='staging'>Staging</SelectItem>
                  <SelectItem value='production'>Production</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </Section>

        {freeOnly ? (
          <Section title='Offer'>
            <FreeOfferSection />
          </Section>
        ) : (
          <>
            <Section title='Plan'>
              <PlanPicker
                selection={selection}
                onSelect={(next) => commit({ plan: next })}
                movedNotice={movedToBusiness ? STARTER_MOVED_NOTICE : undefined}
                closedPlans={closedPlans}
              />
            </Section>

            <Section title='Expected volume'>
              <VolumePicker
                hosting='managed'
                engine='ferriskey'
                volume={volume}
                onChange={(next) => commit({ volume: next })}
              />
            </Section>
          </>
        )}

        {tier && (
          <Section title='Tier'>
            <TierPicker tiers={tiers} selected={tier.code} onSelect={setPickedTier} />
          </Section>
        )}

        {!releasesLoading && version === '' && (
          <p className='rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
            No version of FerrisKey is available yet, so there is nothing to create. Someone
            operating the platform publishes one and makes it available.
          </p>
        )}

        {!offersLoading && open.length === 0 && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            This organisation's plan opens no offer, so there is nothing to deploy on.
          </p>
        )}

        {derivedClosed && (
          <p className='rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
            {OFFER_COPY[derived].label} fits this choice, but it is not available on your plan.
            Choose another plan or volume.
          </p>
        )}

        {refusal && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            {refusal}
          </p>
        )}

        <div className='sticky bottom-0 -mx-2 space-y-4 border-t bg-background px-2 py-4'>
          {freeOnly && !tier ? (
            <div className='flex items-baseline justify-between gap-4 rounded-lg border bg-muted/30 p-4'>
              <span className='text-sm font-medium'>Monthly price</span>
              <span className='text-xl font-semibold'>{FREE_OFFER_COPY.price}</span>
            </div>
          ) : (
            <PriceSummary hosting='managed' price={price} tier={tier} />
          )}
          <CreateActions
            canSubmit={canSubmit}
            block={block}
            isSubmitting={isSubmitting}
            onCancel={() => navigate({ to: organisationPath('/deployments') })}
          />
        </div>
      </form>
    </Page>
  )
}
