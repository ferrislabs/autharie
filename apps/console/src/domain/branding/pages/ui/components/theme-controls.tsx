import { RotateCcw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { defaultTokens, getAt, isHex, type TokenProblems, type Tokens } from '../../../theme-tokens'

export interface PanelContext {
  tokens: Tokens
  problems: TokenProblems
  readOnly: boolean
  set: (path: string, value: unknown) => void
}

const FALLBACK = defaultTokens()

function idOf(path: string): string {
  return `theme-${path.replace(/\./g, '-')}`
}

function sameColour(a: unknown, b: unknown): boolean {
  return String(a).trim().toLowerCase() === String(b).trim().toLowerCase()
}

function Row({
  label,
  path,
  ctx,
  modified,
  children,
}: {
  label: string
  path: string
  ctx: PanelContext
  modified: boolean
  children: React.ReactNode
}) {
  const problem = ctx.problems[path]
  const id = idOf(path)

  return (
    <div className='space-y-1'>
      <div className='flex items-center justify-between gap-3'>
        <Label htmlFor={id} className='text-xs text-muted-foreground'>
          {label}
        </Label>
        <div className='flex items-center gap-1.5'>
          {children}
          {!ctx.readOnly && (
            <Button
              type='button'
              variant='ghost'
              size='icon-sm'
              className='size-6'
              aria-label={`Reset ${label} to the default`}
              title='Reset to the default'
              disabled={!modified}
              onClick={() => ctx.set(path, getAt(FALLBACK, path))}
            >
              <RotateCcw className='size-3' />
            </Button>
          )}
        </div>
      </div>
      {problem && (
        <p id={`${id}-problem`} className='text-right text-xs text-destructive'>
          {problem}
        </p>
      )}
    </div>
  )
}

export function ColorControl({
  label,
  path,
  ctx,
}: {
  label: string
  path: string
  ctx: PanelContext
}) {
  const value = String(getAt(ctx.tokens, path) ?? '')
  const fallback = String(getAt(FALLBACK, path))
  const problem = ctx.problems[path]
  const id = idOf(path)

  return (
    <Row label={label} path={path} ctx={ctx} modified={!sameColour(value, fallback)}>
      <input
        type='color'
        aria-label={`${label} picker`}
        value={isHex(value) ? value.trim().toLowerCase() : fallback}
        disabled={ctx.readOnly}
        onChange={(event) => ctx.set(path, event.target.value)}
        className='h-7 w-9 cursor-pointer rounded-md border bg-transparent p-0.5 disabled:cursor-not-allowed'
      />
      <Input
        id={id}
        value={value}
        readOnly={ctx.readOnly}
        onChange={(event) => ctx.set(path, event.target.value)}
        aria-invalid={!!problem}
        aria-describedby={problem ? `${id}-problem` : undefined}
        className='h-7 w-24 px-2 font-mono text-xs'
      />
    </Row>
  )
}

export function NumberControl({
  label,
  path,
  ctx,
  min,
  max,
  step,
  unit,
}: {
  label: string
  path: string
  ctx: PanelContext
  min: number
  max: number
  step: number
  unit?: string
}) {
  const value = getAt(ctx.tokens, path)
  const problem = ctx.problems[path]
  const id = idOf(path)
  const shown = typeof value === 'number' && !Number.isNaN(value) ? value : ''

  return (
    <Row label={label} path={path} ctx={ctx} modified={!Object.is(value, getAt(FALLBACK, path))}>
      <Input
        id={id}
        type='number'
        min={min}
        max={max}
        step={step}
        value={shown}
        readOnly={ctx.readOnly}
        onChange={(event) =>
          ctx.set(path, event.target.value === '' ? Number.NaN : Number(event.target.value))
        }
        aria-invalid={!!problem}
        aria-describedby={problem ? `${id}-problem` : undefined}
        className='h-7 w-20 px-2 text-right text-xs tabular-nums'
      />
      {unit && <span className='w-5 text-[11px] text-muted-foreground'>{unit}</span>}
    </Row>
  )
}

export function ChoiceControl({
  label,
  path,
  ctx,
  options,
}: {
  label: string
  path: string
  ctx: PanelContext
  options: ReadonlyArray<{ value: string; label: string }>
}) {
  const value = String(getAt(ctx.tokens, path))

  return (
    <Row label={label} path={path} ctx={ctx} modified={value !== getAt(FALLBACK, path)}>
      <Select value={value} onValueChange={(next) => ctx.set(path, next)} disabled={ctx.readOnly}>
        <SelectTrigger id={idOf(path)} className='h-7 w-32 px-2 text-xs'>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Row>
  )
}

export function TextControl({
  label,
  path,
  ctx,
  placeholder,
}: {
  label: string
  path: string
  ctx: PanelContext
  placeholder?: string
}) {
  const value = getAt(ctx.tokens, path)
  const problem = ctx.problems[path]
  const id = idOf(path)

  return (
    <Row label={label} path={path} ctx={ctx} modified={(value ?? null) !== getAt(FALLBACK, path)}>
      <Input
        id={id}
        value={typeof value === 'string' ? value : ''}
        placeholder={placeholder}
        readOnly={ctx.readOnly}
        onChange={(event) => ctx.set(path, event.target.value === '' ? null : event.target.value)}
        aria-invalid={!!problem}
        aria-describedby={problem ? `${id}-problem` : undefined}
        className='h-7 w-56 px-2 text-xs'
      />
    </Row>
  )
}
