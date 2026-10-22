import { Clock } from 'lucide-react'
import { FORBID, MUTED, PERMIT } from './flow-elements'

function Swatch({ color, dashed = false }: { color: string; dashed?: boolean }) {
  return (
    <svg width='22' height='6' aria-hidden>
      <line x1='0' y1='3' x2='22' y2='3' stroke={color} strokeWidth='2.5' strokeDasharray={dashed ? '4 3' : undefined} />
    </svg>
  )
}

function Item({ children, mark }: { children: React.ReactNode; mark: React.ReactNode }) {
  return (
    <span className='flex items-center gap-1.5'>
      {mark}
      {children}
    </span>
  )
}

export function CanvasLegend({ canManage }: { canManage: boolean }) {
  return (
    <div className='flex flex-wrap items-center justify-between gap-3'>
      <div className='flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground'>
        <Item mark={<Swatch color={PERMIT} />}>Allow</Item>
        <Item mark={<Swatch color={FORBID} />}>Deny</Item>
        <Item mark={<Swatch color={MUTED} dashed />}>Disabled</Item>
        <Item mark={<Clock className='size-3' />}>Condition</Item>
      </div>
      <p className='text-xs text-muted-foreground'>
        {canManage
          ? 'Right-click to add, drag between boxes to link, click a box to edit.'
          : 'Click a box to see its details.'}
      </p>
    </div>
  )
}
