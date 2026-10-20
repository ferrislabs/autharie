import { useState } from 'react'
import { formatDistanceToNow } from 'date-fns'
import type { Schemas } from '@/api/api.client'
import { Card } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { NO_OWNER, choiceFromOwner, ownerFromChoice } from '../../cells'

interface Props {
  settings?: Schemas.CellsSettingsView
  organisations: Schemas.Tenant[]
  isLoading: boolean
  canOperate: boolean
  liveCells: number
  onSave: (owner: string | null) => void
  isSaving: boolean
  refusal?: string
}

export function CellsSettingsCard({
  settings,
  organisations,
  isLoading,
  canOperate,
  liveCells,
  onSave,
  isSaving,
  refusal,
}: Props) {
  const [picked, setPicked] = useState<string | undefined>(undefined)

  if (isLoading || !settings) {
    return <Skeleton className='h-40 w-full' />
  }

  const saved = choiceFromOwner(settings.organisation_id)
  const choice = picked ?? saved
  const unchanged = choice === saved
  const owner = organisations.find((tenant) => tenant.organisation.id === settings.organisation_id)

  return (
    <Card className='space-y-4'>
      <div className='space-y-1'>
        <h2 className='text-base font-semibold'>Cells organisation</h2>
        <p className='text-sm text-muted-foreground'>
          The organisation that owns the FerrisKey instance of every cell. Without one, no cell can
          be opened.
        </p>
      </div>

      <div className='grid gap-2 sm:max-w-md'>
        <Label htmlFor='cells-owner'>Owner</Label>
        <Select value={choice} onValueChange={setPicked} disabled={!canOperate || isSaving}>
          <SelectTrigger id='cells-owner'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_OWNER}>None</SelectItem>
            {organisations.map((tenant) => (
              <SelectItem key={tenant.organisation.id} value={tenant.organisation.id}>
                {tenant.organisation.name}
              </SelectItem>
            ))}
            {settings.organisation_id && !owner && (
              <SelectItem value={settings.organisation_id}>{settings.organisation_id}</SelectItem>
            )}
          </SelectContent>
        </Select>
      </div>

      {liveCells > 0 && (
        <p className='text-sm text-muted-foreground'>
          {liveCells} cell(s) are not retired. The owner can only change once every cell is retired,
          because their instances stay owned by the organisation that held them.
        </p>
      )}

      {refusal && (
        <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
          {refusal}
        </p>
      )}

      <div className='flex flex-wrap items-center justify-between gap-3'>
        <p className='text-xs text-muted-foreground'>
          {settings.updated_by
            ? `Set by ${settings.updated_by} ${formatDistanceToNow(new Date(settings.updated_at))} ago`
            : 'Never set'}
        </p>
        {canOperate && (
          <Button
            size='sm'
            disabled={unchanged || isSaving}
            onClick={() => {
              onSave(ownerFromChoice(choice))
              setPicked(undefined)
            }}
          >
            {isSaving ? 'Saving…' : 'Save'}
          </Button>
        )}
      </div>
    </Card>
  )
}
