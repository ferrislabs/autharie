import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { OVERRIDE_CHOICES, choiceOf, stateLabel } from '../../choice'
import type { Feature, OverrideChoice, TenantFeature } from '../../types/feature'

interface Props {
  features: TenantFeature[]
  isLoading: boolean
  canChange: boolean
  pending?: Feature
  refusal?: string
  onChoose: (feature: Feature, choice: OverrideChoice) => void
}

const CHOICE_LABELS: Record<OverrideChoice, string> = {
  default: 'Default',
  on: 'On',
  off: 'Off',
}

export function TenantFeatures({
  features,
  isLoading,
  canChange,
  pending,
  refusal,
  onChoose,
}: Props) {
  return (
    <section className='mt-10'>
      <h2 className='text-lg font-semibold'>Features</h2>

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
          features.map((feature) => {
            const current = choiceOf(feature.override)

            return (
              <div
                key={feature.key}
                className='flex flex-wrap items-center justify-between gap-4 px-4 py-3'
              >
                <div className='min-w-0'>
                  <p className='font-mono text-sm font-medium'>{feature.key}</p>
                  <p className='text-xs text-muted-foreground'>{feature.description}</p>
                  <p className='mt-1 text-xs text-muted-foreground'>
                    Default {stateLabel(feature.default).toLowerCase()}, effective{' '}
                    <span className='font-medium text-foreground'>
                      {stateLabel(feature.effective).toLowerCase()}
                    </span>
                  </p>
                </div>
                <div
                  className='inline-flex gap-1'
                  role='group'
                  aria-label={`Override for ${feature.key}`}
                >
                  {OVERRIDE_CHOICES.map((choice) => (
                    <Button
                      key={choice}
                      size='sm'
                      variant={choice === current ? 'default' : 'outline'}
                      aria-pressed={choice === current}
                      disabled={!canChange || pending === feature.key}
                      onClick={() => choice !== current && onChoose(feature.key, choice)}
                    >
                      {CHOICE_LABELS[choice]}
                    </Button>
                  ))}
                </div>
              </div>
            )
          })
        )}
      </div>
    </section>
  )
}
