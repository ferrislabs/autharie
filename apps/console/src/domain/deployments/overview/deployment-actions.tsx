import { useState } from 'react'
import { ChevronDown, Play, Square } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Spinner } from '@/components/ui/spinner'
import { powerItems, STOP_CONFIRMATION, type PowerAction } from '../power'
import type { DeploymentStatus } from '../types/deployment'

export interface DeploymentPower {
  status: DeploymentStatus
  busy: boolean
  refusal?: string
  onStop: () => void
  onStart: () => void
}

const ICONS: Record<PowerAction, typeof Play> = { stop: Square, start: Play }

export function DeploymentActions({
  status,
  busy,
  onStop,
  onStart,
}: Pick<DeploymentPower, 'status' | 'busy' | 'onStop' | 'onStart'>) {
  const [confirming, setConfirming] = useState(false)

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button variant='outline' size='sm' disabled={busy}>
            {busy && <Spinner className='size-3.5' />}
            Actions
            <ChevronDown className='h-4 w-4' />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align='end'>
          {powerItems(status, busy).map((item) => {
            const Icon = ICONS[item.action]

            return (
              <DropdownMenuItem
                key={item.action}
                disabled={item.disabled}
                onSelect={() => (item.action === 'stop' ? setConfirming(true) : onStart())}
              >
                <Icon className='h-4 w-4' />
                {item.label}
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent className='sm:max-w-md'>
          <DialogHeader>
            <DialogTitle>{STOP_CONFIRMATION.title}</DialogTitle>
            <DialogDescription>{STOP_CONFIRMATION.description}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type='button' variant='ghost' onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            <Button
              type='button'
              variant='destructive'
              onClick={() => {
                setConfirming(false)
                onStop()
              }}
            >
              {STOP_CONFIRMATION.confirm}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
