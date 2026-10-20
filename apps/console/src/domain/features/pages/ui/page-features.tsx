import { EmptyState, Page, PageTitle } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Flag } from 'lucide-react'
import type { Feature, FeatureOverview } from '../../types/feature'

interface Props {
  features: FeatureOverview[]
  isLoading: boolean
  canChange: boolean
  pending?: Feature
  refusal?: string
  onToggle: (feature: Feature, enabled: boolean) => void
}

export function PageFeatures({
  features,
  isLoading,
  canChange,
  pending,
  refusal,
  onToggle,
}: Props) {
  return (
    <Page>
      <PageTitle icon={<Flag className='h-5 w-5 text-muted-foreground' />} title='Features' />

      <p className='mt-4 text-sm text-muted-foreground'>
        The default applies to every organisation without an override of its own. Overrides are set
        from the organisation&apos;s page.
      </p>

      {refusal && (
        <div
          className='mt-4 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm'
          role='alert'
        >
          {refusal}
        </div>
      )}

      <div className='mt-6'>
        {isLoading ? (
          <Skeleton className='h-24' />
        ) : features.length === 0 ? (
          <EmptyState
            icon={<Flag className='h-5 w-5' />}
            title='No features'
            description='This installation defines no feature flags.'
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Feature</TableHead>
                <TableHead className='text-right'>Overrides on</TableHead>
                <TableHead className='text-right'>Overrides off</TableHead>
                <TableHead className='text-right'>Default</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {features.map((feature) => (
                <TableRow key={feature.key}>
                  <TableCell>
                    <p className='font-mono text-sm font-medium'>{feature.key}</p>
                    <p className='text-xs text-muted-foreground'>{feature.description}</p>
                  </TableCell>
                  <TableCell className='text-right tabular-nums'>{feature.overrides_on}</TableCell>
                  <TableCell className='text-right tabular-nums'>{feature.overrides_off}</TableCell>
                  <TableCell className='text-right'>
                    <Switch
                      aria-label={`Default for ${feature.key}`}
                      checked={feature.enabled_by_default}
                      disabled={!canChange || pending === feature.key}
                      onCheckedChange={(enabled) => onToggle(feature.key, enabled)}
                    />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>
    </Page>
  )
}
