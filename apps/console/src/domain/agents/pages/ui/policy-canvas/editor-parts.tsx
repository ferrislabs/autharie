import { useState } from 'react'
import { X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'
import { splitList } from '../../../rule-form'

export function EditorShell({
  title,
  subtitle,
  onClose,
  children,
}: {
  title: string
  subtitle?: string
  onClose: () => void
  children: React.ReactNode
}) {
  return (
    <>
      <div className='flex items-start justify-between gap-2 border-b px-4 py-3'>
        <div className='min-w-0'>
          <p className='text-sm font-semibold'>{title}</p>
          {subtitle && <p className='text-xs text-muted-foreground'>{subtitle}</p>}
        </div>
        <Button type='button' variant='ghost' size='icon-sm' aria-label='Close editor' onClick={onClose}>
          <X className='size-4' />
        </Button>
      </div>
      <div className='space-y-4 overflow-y-auto px-4 py-3'>{children}</div>
    </>
  )
}

export function Row({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: string
  htmlFor?: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className='space-y-1.5'>
      <Label htmlFor={htmlFor} className='text-xs'>
        {label}
      </Label>
      {children}
      {hint && <p className='text-xs text-muted-foreground'>{hint}</p>}
    </div>
  )
}

export function Segmented<T extends string>({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string
  value: T
  options: { value: T; label: string; tone?: 'permit' | 'forbid' }[]
  disabled?: boolean
  onChange: (value: T) => void
}) {
  return (
    <div role='radiogroup' aria-label={label} className='grid auto-cols-fr grid-flow-col gap-1 rounded-md bg-muted p-1'>
      {options.map((option) => {
        const active = option.value === value
        return (
          <button
            key={option.value}
            type='button'
            role='radio'
            aria-checked={active}
            disabled={disabled}
            className={cn(
              'rounded-sm px-2 py-1 text-xs font-medium text-muted-foreground transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed',
              active && 'bg-background text-foreground shadow-xs',
              active && option.tone === 'permit' && 'text-emerald-700 dark:text-emerald-400',
              active && option.tone === 'forbid' && 'text-destructive',
            )}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}

export function ListInput({
  id,
  values,
  placeholder,
  disabled,
  onChange,
}: {
  id: string
  values: string[]
  placeholder: string
  disabled?: boolean
  onChange: (values: string[]) => void
}) {
  const [text, setText] = useState(values.join(', '))

  return (
    <Input
      id={id}
      value={text}
      placeholder={placeholder}
      disabled={disabled}
      spellCheck={false}
      onChange={(event) => {
        setText(event.target.value)
        onChange(splitList(event.target.value))
      }}
    />
  )
}

export const TEXTAREA =
  'flex min-h-16 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50'

export function SharedNote({ count, what }: { count: number; what: string }) {
  if (count < 2) return null
  return (
    <p className='rounded-md border bg-muted/40 px-2.5 py-1.5 text-xs text-muted-foreground'>
      This {what} is linked to {count} rules. Changing it changes all of them.
    </p>
  )
}
