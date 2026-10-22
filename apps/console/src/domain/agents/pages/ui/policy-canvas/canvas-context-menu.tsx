import { ChevronRight, type LucideIcon } from 'lucide-react'
import { Fragment, useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

export interface MenuItem {
  id: string
  label: string
  icon?: LucideIcon
  onSelect?: () => void
  items?: MenuItem[]
  destructive?: boolean
  disabled?: boolean
  separatorBefore?: boolean
}

interface OpenSub {
  id: string
  keyboard: boolean
}

const MENU_WIDTH = 224

const ITEM =
  'flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm outline-none hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground disabled:pointer-events-none disabled:opacity-50'

function rowsOf(list: HTMLElement | null): HTMLButtonElement[] {
  if (!list) return []
  return Array.from(list.querySelectorAll<HTMLButtonElement>(':scope > div > [role="menuitem"]')).filter(
    (button) => !button.disabled,
  )
}

function MenuList({
  items,
  label,
  flip,
  up,
  autoFocus,
  className,
  onClose,
  onBack,
}: {
  items: MenuItem[]
  label?: string
  flip: boolean
  up: boolean
  autoFocus: boolean
  className?: string
  onClose: () => void
  onBack?: () => void
}) {
  const list = useRef<HTMLDivElement>(null)
  const buttons = useRef<Record<string, HTMLButtonElement | null>>({})
  const [open, setOpen] = useState<OpenSub | null>(null)

  useEffect(() => {
    if (autoFocus) rowsOf(list.current)[0]?.focus()
  }, [autoFocus])

  function move(step: number) {
    const rows = rowsOf(list.current)
    if (rows.length === 0) return
    const current = rows.findIndex((row) => row === document.activeElement)
    const next = current < 0 ? (step > 0 ? 0 : rows.length - 1) : (current + step + rows.length) % rows.length
    rows[next].focus()
  }

  function onKeyDown(event: React.KeyboardEvent) {
    const active = document.activeElement
    const focused = items.find((item) => buttons.current[item.id] === active)
    switch (event.key) {
      case 'ArrowDown':
        move(1)
        break
      case 'ArrowUp':
        move(-1)
        break
      case 'ArrowRight':
        if (!focused?.items || focused.disabled) return
        setOpen({ id: focused.id, keyboard: true })
        break
      case 'ArrowLeft':
        if (!onBack) return
        onBack()
        break
      case 'Escape':
        onClose()
        break
      default:
        return
    }
    event.preventDefault()
    event.stopPropagation()
  }

  function closeSub(id: string) {
    setOpen(null)
    buttons.current[id]?.focus()
  }

  return (
    <div
      ref={list}
      role='menu'
      aria-label={label}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      className={cn(
        'z-50 min-w-52 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md',
        className,
      )}
      style={{ width: MENU_WIDTH }}
    >
      {items.map((item) => (
        <Fragment key={item.id}>
          {item.separatorBefore && <div role='separator' className='-mx-1 my-1 h-px bg-border' />}
          <div
            className='relative'
            onMouseEnter={() => {
              if (item.items && !item.disabled) setOpen({ id: item.id, keyboard: false })
              else setOpen(null)
            }}
          >
            <button
              type='button'
              role='menuitem'
              ref={(node) => {
                buttons.current[item.id] = node
              }}
              disabled={item.disabled}
              aria-haspopup={item.items ? 'menu' : undefined}
              aria-expanded={item.items ? open?.id === item.id : undefined}
              className={cn(ITEM, item.destructive && 'text-destructive hover:bg-destructive/10 hover:text-destructive focus-visible:bg-destructive/10 focus-visible:text-destructive')}
              onClick={() => {
                if (item.items) {
                  setOpen({ id: item.id, keyboard: true })
                  return
                }
                item.onSelect?.()
                onClose()
              }}
            >
              {item.icon && <item.icon className='size-4 shrink-0 text-muted-foreground' />}
              <span className='flex-1 truncate'>{item.label}</span>
              {item.items && <ChevronRight className='size-4 shrink-0 text-muted-foreground' />}
            </button>
            {item.items && open?.id === item.id && (
              <MenuList
                items={item.items}
                label={item.label}
                flip={flip}
                up={up}
                autoFocus={open.keyboard}
                className={cn('absolute', up ? 'bottom-0' : 'top-0', flip ? 'right-full mr-1' : 'left-full ml-1')}
                onClose={onClose}
                onBack={() => closeSub(item.id)}
              />
            )}
          </div>
        </Fragment>
      ))}
    </div>
  )
}

export function CanvasContextMenu({
  x,
  y,
  items,
  width,
  height,
  label,
  onClose,
}: {
  x: number
  y: number
  items: MenuItem[]
  width: number
  height: number
  label?: string
  onClose: () => void
}) {
  const root = useRef<HTMLDivElement>(null)
  const [menuHeight, setMenuHeight] = useState(0)

  function measure(element: HTMLDivElement | null) {
    root.current = element
    if (element && element.offsetHeight !== menuHeight) setMenuHeight(element.offsetHeight)
  }

  useEffect(() => {
    function onPointerDown(event: PointerEvent) {
      if (root.current && event.target instanceof Node && !root.current.contains(event.target)) onClose()
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  }, [onClose])

  const left = Math.max(4, Math.min(x, width - MENU_WIDTH - 4))
  const top = Math.max(4, Math.min(y, height - menuHeight - 4))
  const up = top + menuHeight / 2 > height / 2
  const flip = left + MENU_WIDTH * 2 + 8 > width

  return (
    <div
      ref={measure}
      className='absolute z-50'
      style={{ left, top }}
      onContextMenu={(event) => event.preventDefault()}
    >
      <MenuList items={items} label={label} flip={flip} up={up} autoFocus onClose={onClose} />
    </div>
  )
}
