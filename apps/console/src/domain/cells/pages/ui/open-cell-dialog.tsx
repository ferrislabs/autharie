import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { regionToOpen } from '../../cells'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onOpenCell: (region: string) => void
  isOpening: boolean
  refusal?: string
}

export function OpenCellDialog({ open, onOpenChange, onOpenCell, isOpening, refusal }: Props) {
  const [typed, setTyped] = useState('')
  const region = regionToOpen(typed)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Open a cell</DialogTitle>
          <DialogDescription>
            Creates a highly available FerrisKey instance on a shared data plane of this region. The
            cell opens once the instance serves two replicas on two database instances.
          </DialogDescription>
        </DialogHeader>

        <div className='grid gap-2'>
          <Label htmlFor='cell-region'>Region</Label>
          <Input
            id='cell-region'
            value={typed}
            placeholder='fr-par'
            onChange={(event) => setTyped(event.target.value)}
          />
        </div>

        {refusal && (
          <p className='rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'>
            {refusal}
          </p>
        )}

        <DialogFooter>
          <Button variant='outline' onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!region || isOpening} onClick={() => region && onOpenCell(region)}>
            {isOpening ? 'Opening…' : 'Open cell'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
