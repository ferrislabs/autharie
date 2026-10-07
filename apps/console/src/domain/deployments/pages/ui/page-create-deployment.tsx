import type { Schemas } from '@/api/api.client'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { newestInstallable } from '@/domain/releases/catalogue'
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
import { Link, useNavigate } from '@tanstack/react-router'
import { useMemo, useState, type ReactNode } from 'react'
import { KIND_LABELS, type DeploymentKind, type Environment } from '../../types/deployment'
import { type Offer, type OfferAvailability } from '../../offers'
import { offerFor, resolveOffer } from '../../pricing/offer-for'
import { priceOf, type Hosting, type PlanId, type Selection } from '../../pricing/model'
import { applyChange, STARTER_MOVED_NOTICE } from '../../pricing/selection'
import { customerCloudAvailability } from '@/domain/cloud-providers/distribution'
import { EnginePicker } from './components/engine-picker'
import { HostingPicker } from './components/hosting-picker'
import { KeycloakNotice } from './components/keycloak-notice'
import { PlanPicker } from './components/plan-picker'
import { PriceSummary } from './components/price-summary'
import { VolumePicker } from './components/volume-picker'

interface Props {
  onSubmit: (data: {
    name: string
    kind: DeploymentKind
    version: string
    environment: Environment
    offer: Offer
    distribution?: Schemas.DistributionRequest
  }) => void
  isSubmitting?: boolean
  /** What the platform said when the last attempt failed. */
  refusal?: string
  offers: OfferAvailability[]
  offersLoading: boolean
  releases: Record<DeploymentKind, Schemas.Release[]>
  releasesLoading: boolean
  credentials: Schemas.CloudCredentialResponse[]
  credentialsLoading: boolean
  customerCloud: ReactNode
  distribution: Schemas.DistributionRequest | null
  clusterEstimate?: string
}

export default function PageCreateDeployment({
  onSubmit,
  isSubmitting = false,
  refusal,
  offers,
  offersLoading,
  releases,
  releasesLoading,
  credentials,
  credentialsLoading,
  customerCloud,
  distribution,
  clusterEstimate,
}: Props) {
  const navigate = useNavigate()
  const organisationPath = useOrganisationPath()

  const [name, setName] = useState('')
  const [kind, setKind] = useState<DeploymentKind>('ferriskey')
  const [environment, setEnvironment] = useState<Environment>('development')
  const [wantsCustomerCloud, setWantsCustomerCloud] = useState(false)
  const [plan, setPlan] = useState<PlanId>('business')
  const [volume, setVolume] = useState(10000)
  const [movedToBusiness, setMovedToBusiness] = useState(false)

  const version = newestInstallable(releases[kind]) ?? ''

  const availability = customerCloudAvailability(kind, credentials)
  const onCustomerCloud = wantsCustomerCloud && availability !== 'hidden'
  const placement = onCustomerCloud ? (distribution ?? undefined) : undefined
  const hosting: Hosting = onCustomerCloud ? 'byoc' : 'managed'

  const selection = useMemo<Selection>(
    () => ({ hosting, engine: kind, plan, volume }),
    [hosting, kind, plan, volume],
  )
  const price = priceOf(selection)

  const derived = offerFor(plan, hosting === 'byoc' ? null : volume)
  const resolved = resolveOffer(derived, offers)
  const chosen = resolved.offer

  const commit = (patch: Partial<Selection>) => {
    const change = applyChange(selection, patch)
    setPlan(change.selection.plan)
    setVolume(change.selection.volume)
    setMovedToBusiness(change.movedToBusiness)
    if (patch.engine) setKind(patch.engine)
    if (patch.hosting) setWantsCustomerCloud(patch.hosting === 'byoc')
  }

  const canSubmit =
    !!chosen &&
    name.trim() !== '' &&
    version !== '' &&
    !isSubmitting &&
    (!onCustomerCloud || (availability === 'available' && !!placement))

  return (
    <Page className='max-w-3xl'>
      <PageTitle title='New deployment' />

      <form
        className='mt-8 space-y-8'
        onSubmit={(e) => {
          e.preventDefault()
          if (chosen) {
            onSubmit({
              name,
              kind,
              version,
              environment,
              offer: chosen,
              ...(placement ? { distribution: placement } : {}),
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

        <Section title='Identity provider'>
          <EnginePicker engine={kind} onSelect={(engine) => commit({ engine })} />

          {kind === 'keycloak' && (
            <KeycloakNotice hosting={hosting} onSwitch={() => commit({ engine: 'ferriskey' })} />
          )}

          <div className='mt-4 space-y-2'>
            <Label>Version</Label>
            {releasesLoading ? (
              <Skeleton className='h-9 w-full sm:w-72' />
            ) : version === '' ? (
              <p className='rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
                No version of {KIND_LABELS[kind]} is available yet, so there is nothing to create.
                Someone operating the platform publishes one and makes it available.
              </p>
            ) : (
              <>
                <p className='font-mono text-sm'>{version}</p>
                <p className='text-xs text-muted-foreground'>
                  The newest available version, chosen for you. It can be upgraded afterwards,
                  which is where choosing a version belongs.
                </p>
              </>
            )}
          </div>
        </Section>

        <Section title='Hosting'>
          <HostingPicker
            hosting={hosting}
            onSelect={(next) => commit({ hosting: next })}
            customerCloudAvailable={availability !== 'hidden'}
          />

          {onCustomerCloud &&
            (credentialsLoading ? (
              <Skeleton className='h-24 w-full' />
            ) : availability === 'needs_credentials' ? (
              <p className='rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground'>
                No cloud account is registered yet.{' '}
                <Link
                  to={organisationPath('/cloud-accounts')}
                  className='font-medium text-foreground underline'
                >
                  Add one on the Cloud accounts page
                </Link>{' '}
                to run a deployment in your own cloud.
              </p>
            ) : (
              customerCloud
            ))}
        </Section>

        <Section title='Plan'>
          <PlanPicker
            selection={selection}
            onSelect={(next) => commit({ plan: next })}
            movedNotice={movedToBusiness ? STARTER_MOVED_NOTICE : undefined}
          />
        </Section>

        <Section title='Expected volume'>
          <VolumePicker
            hosting={hosting}
            engine={kind}
            volume={volume}
            onChange={(next) => commit({ volume: next })}
          />
        </Section>

        {!offersLoading && offers.length === 0 && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            This organisation's plan opens no offer, so there is nothing to deploy on.
          </p>
        )}

        {resolved.because && (
          <p className='rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'>
            {resolved.because}
          </p>
        )}

        {refusal && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            {refusal}
          </p>
        )}

        <div className='sticky bottom-0 -mx-2 space-y-4 border-t bg-background px-2 py-4'>
          <PriceSummary hosting={hosting} price={price} clusterEstimate={clusterEstimate} />
          <div className='flex items-center justify-end gap-2'>
            <Button
              type='button'
              variant='ghost'
              onClick={() => navigate({ to: organisationPath('/deployments') })}
            >
              Cancel
            </Button>
            <Button type='submit' disabled={!canSubmit}>
              {isSubmitting ? 'Creating…' : 'Create deployment'}
            </Button>
          </div>
        </div>
      </form>
    </Page>
  )
}
