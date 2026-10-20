import { useState } from 'react'
import { formatDistanceToNow } from 'date-fns'
import { Plus, Rows3 } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState, Page, PageTitle, Section } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Meter } from '@/components/ui/meter'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import {
  canDrain,
  canRetire,
  cellStatusLabel,
  cellStatusTone,
  liveCells,
  occupancy,
} from '../../cells'
import { CellsSettingsCard } from './cells-settings-card'
import { OpenCellDialog } from './open-cell-dialog'

type Confirmation = { kind: 'drain' | 'retire'; cell: Schemas.CellView }

interface Props {
  cells: Schemas.CellView[]
  settings?: Schemas.CellsSettingsView
  organisations: Schemas.Tenant[]
  isLoading: boolean
  canOperate: boolean
  onSaveOwner: (owner: string | null) => void
  isSavingOwner: boolean
  ownerRefusal?: string
  onOpenCell: (region: string, onDone: () => void) => void
  isOpening: boolean
  openRefusal?: string
  onDrain: (cellId: string) => void
  onRetire: (cellId: string) => void
  pendingCellId?: string
  actionRefusal?: string
}

export function PageCells({
  cells,
  settings,
  organisations,
  isLoading,
  canOperate,
  onSaveOwner,
  isSavingOwner,
  ownerRefusal,
  onOpenCell,
  isOpening,
  openRefusal,
  onDrain,
  onRetire,
  pendingCellId,
  actionRefusal,
}: Props) {
  const [opening, setOpening] = useState(false)
  const [confirming, setConfirming] = useState<Confirmation | undefined>(undefined)

  const openAction = canOperate && (
    <Button size='sm' onClick={() => setOpening(true)}>
      <Plus className='h-4 w-4' />
      Open a cell
    </Button>
  )

  return (
    <Page>
      <PageTitle title='Cells' actions={openAction} />

      <div className='mt-6 space-y-8'>
        <CellsSettingsCard
          settings={settings}
          organisations={organisations}
          isLoading={isLoading}
          canOperate={canOperate}
          liveCells={liveCells(cells)}
          onSave={onSaveOwner}
          isSaving={isSavingOwner}
          refusal={ownerRefusal}
        />

        {actionRefusal && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            {actionRefusal}
          </p>
        )}

        <Section title='Cells'>
          {isLoading ? (
            <Skeleton className='h-32 w-full' />
          ) : cells.length === 0 ? (
            <EmptyState
              icon={<Rows3 className='h-5 w-5' />}
              title='No cell yet'
              description='Free tenants get a realm on a shared cell. Choose the cells organisation, then open a cell in a region served by a shared data plane.'
              action={openAction}
            />
          ) : (
            <div className='overflow-x-auto rounded-lg border'>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Region</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Realms</TableHead>
                    <TableHead>Created</TableHead>
                    {canOperate && <TableHead className='text-right'>Actions</TableHead>}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {cells.map((cell) => (
                    <TableRow key={cell.id}>
                      <TableCell className='font-medium'>
                        {cell.region}
                        <span className='ml-2 font-mono text-xs text-muted-foreground'>
                          {cell.id.slice(0, 8)}
                        </span>
                      </TableCell>
                      <TableCell>
                        <StatusBadge tone={cellStatusTone(cell.status)}>
                          {cellStatusLabel(cell.status)}
                        </StatusBadge>
                      </TableCell>
                      <TableCell className='min-w-40'>
                        <div className='flex items-center gap-2'>
                          <span className='w-16 shrink-0 font-mono text-xs'>
                            {cell.realms} / {cell.capacity}
                          </span>
                          <Meter percent={occupancy(cell)} />
                        </div>
                      </TableCell>
                      <TableCell className='text-muted-foreground'>
                        {formatDistanceToNow(new Date(cell.created_at))} ago
                      </TableCell>
                      {canOperate && (
                        <TableCell className='text-right'>
                          <div className='flex justify-end gap-2'>
                            {canDrain(cell.status) && (
                              <Button
                                variant='outline'
                                size='sm'
                                disabled={pendingCellId === cell.id}
                                onClick={() => setConfirming({ kind: 'drain', cell })}
                              >
                                Drain
                              </Button>
                            )}
                            {canRetire(cell) && (
                              <Button
                                variant='outline'
                                size='sm'
                                disabled={pendingCellId === cell.id}
                                onClick={() => setConfirming({ kind: 'retire', cell })}
                              >
                                Retire
                              </Button>
                            )}
                          </div>
                        </TableCell>
                      )}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </Section>
      </div>

      <OpenCellDialog
        open={opening}
        onOpenChange={setOpening}
        onOpenCell={(region) => onOpenCell(region, () => setOpening(false))}
        isOpening={isOpening}
        refusal={openRefusal}
      />

      <Dialog open={!!confirming} onOpenChange={(next) => !next && setConfirming(undefined)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirming?.kind === 'drain' ? 'Drain this cell?' : 'Retire this cell?'}
            </DialogTitle>
            <DialogDescription>
              {confirming?.kind === 'drain'
                ? 'The cell stops receiving new tenants. The tenants it holds stay.'
                : 'The cell is retired for good. This cannot be undone, and its instance can then be deleted.'}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant='outline' onClick={() => setConfirming(undefined)}>
              Cancel
            </Button>
            <Button
              variant={confirming?.kind === 'retire' ? 'destructive' : 'default'}
              onClick={() => {
                if (confirming?.kind === 'drain') onDrain(confirming.cell.id)
                if (confirming?.kind === 'retire') onRetire(confirming.cell.id)
                setConfirming(undefined)
              }}
            >
              {confirming?.kind === 'drain' ? 'Drain' : 'Retire'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Page>
  )
}
