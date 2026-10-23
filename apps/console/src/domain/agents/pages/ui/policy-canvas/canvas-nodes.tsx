import { Handle, Position, type NodeProps } from '@xyflow/react'
import { AlertTriangle, Bot, Clock, Eye, Globe, KeyRound, Server, User, Users, Wrench } from 'lucide-react'
import { cn } from '@/lib/utils'
import { COLUMN_WIDTH, type RuleBox, type Subject, type Target } from '../../../canvas-model'
import {
  conditionLabel,
  destructiveCount,
  isDestructiveTarget,
  subjectCaption,
  subjectLabel,
  targetCaption,
  targetLabel,
} from '../../../canvas-labels'
import { useCanvasUi, useDimmedBox } from './canvas-context'
import {
  MUTED,
  strokeOf,
  type HeaderNodeType,
  type RuleNodeType,
  type WhatNodeType,
  type WhoNodeType,
} from './flow-elements'

const HANDLE =
  'size-2.5! border-2! border-background! bg-primary! opacity-60 transition-opacity group-hover:opacity-100 [&.connectingto]:opacity-100'

const CARD =
  'group relative rounded-lg border bg-card text-card-foreground shadow-xs transition-opacity'

function SubjectIcon({ subject }: { subject: Subject }) {
  const className = 'size-4'
  switch (subject.type) {
    case 'any':
      return <Globe className={className} />
    case 'group':
      return <Users className={className} />
    case 'role':
      return <KeyRound className={className} />
    case 'user':
      return <User className={className} />
    case 'agent':
      return <Bot className={className} />
  }
}

function TargetIcon({ target }: { target: Target }) {
  const className = 'size-4'
  if (target.type === 'read-only') return <Eye className={className} />
  if (target.type === 'server') return <Server className={className} />
  return <Wrench className={className} />
}

function IconTile({ children, tone }: { children: React.ReactNode; tone?: 'destructive' | 'primary' }) {
  return (
    <div
      className={cn(
        'flex size-8 shrink-0 items-center justify-center rounded-md border bg-muted text-muted-foreground',
        tone === 'destructive' && 'border-destructive/30 bg-destructive/10 text-destructive',
        tone === 'primary' && 'border-primary/30 bg-primary/10 text-primary',
      )}
    >
      {children}
    </div>
  )
}

export function WhoNode({ id, data, selected }: NodeProps<WhoNodeType>) {
  const { readOnly } = useCanvasUi()
  const dimmed = useDimmedBox(id)
  const subject = data.box.subject
  const empty = subject.type !== 'any' && subject.value.trim().length === 0

  return (
    <div
      className={cn(CARD, 'flex items-center gap-3 px-3 py-2', selected && 'ring-2 ring-ring', empty && 'border-dashed')}
      style={{ width: COLUMN_WIDTH.who, opacity: dimmed ? 0.25 : 1 }}
    >
      <IconTile tone={subject.type === 'agent' ? 'primary' : undefined}>
        <SubjectIcon subject={subject} />
      </IconTile>
      <div className='min-w-0 flex-1'>
        <p className={cn('truncate text-sm font-medium', empty && 'text-muted-foreground')}>
          {subjectLabel(subject)}
        </p>
        <p className='truncate text-xs text-muted-foreground'>{subjectCaption(subject)}</p>
      </div>
      <Handle type='source' position={Position.Right} className={HANDLE} isConnectable={!readOnly} />
    </div>
  )
}

export function WhatNode({ id, data, selected }: NodeProps<WhatNodeType>) {
  const { readOnly } = useCanvasUi()
  const dimmed = useDimmedBox(id)
  const target = data.box.target
  const destructive = isDestructiveTarget(target)
  const some = destructiveCount(target)

  return (
    <div
      className={cn(CARD, 'flex items-center gap-3 px-3 py-2', selected && 'ring-2 ring-ring', destructive && 'border-destructive/50')}
      style={{ width: COLUMN_WIDTH.what, opacity: dimmed ? 0.25 : 1 }}
    >
      <Handle
        type='target'
        position={Position.Left}
        className={HANDLE}
        isConnectable={!readOnly}
        isConnectableStart={false}
      />
      <IconTile tone={destructive ? 'destructive' : undefined}>
        <TargetIcon target={target} />
      </IconTile>
      <div className='min-w-0 flex-1'>
        <p className='truncate text-sm font-medium' title={target.type === 'tools' ? target.tools.join('\n') : undefined}>
          {targetLabel(target)}
        </p>
        <p className='flex items-center gap-1 truncate text-xs text-muted-foreground'>
          {some > 0 && <AlertTriangle className='size-3 shrink-0 text-destructive' />}
          <span className='truncate'>
            {some > 0 && !destructive ? `${some} destructive · ` : destructive ? 'destructive · ' : ''}
            {targetCaption(target)}
          </span>
        </p>
      </div>
    </div>
  )
}

function Chip({ children, icon, className }: { children: React.ReactNode; icon?: React.ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex max-w-full items-center gap-1 rounded-full border bg-muted/50 px-2 py-0.5 text-[11px] text-muted-foreground',
        className,
      )}
    >
      {icon}
      <span className='truncate'>{children}</span>
    </span>
  )
}

function agentsChip(box: RuleBox): string {
  if (box.agents.kind === 'any') return 'via Any agent'
  const ids = box.agents.clientIds.filter((id) => id.trim().length > 0)
  return ids.length === 0 ? 'via no agent yet' : `via ${ids.join(', ')}`
}

export function RuleNode({ id, data, selected }: NodeProps<RuleNodeType>) {
  const { readOnly, status } = useCanvasUi()
  const dimmed = useDimmedBox(id)
  const box = data.box
  const state = status[id]
  const issues = state?.issues.length ?? 0
  const color = strokeOf(box.effect, true)
  const condition = conditionLabel(box.when)

  return (
    <div
      className={cn(
        CARD,
        'overflow-hidden border-l-4 px-3 py-2.5',
        selected && 'ring-2 ring-ring',
        !box.enabled && 'border-dashed bg-muted/40',
      )}
      style={{
        width: COLUMN_WIDTH.rule,
        opacity: dimmed ? 0.25 : box.enabled ? 1 : 0.75,
        borderLeftColor: box.enabled ? color : MUTED,
      }}
    >
      <Handle
        type='target'
        position={Position.Left}
        className={HANDLE}
        isConnectable={!readOnly}
        isConnectableStart={false}
      />
      <div className='flex items-center gap-2'>
        <span
          className='shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white'
          style={{ background: box.enabled ? color : MUTED }}
        >
          {box.effect === 'permit' ? 'Allow' : 'Deny'}
        </span>
        <p className={cn('min-w-0 flex-1 truncate text-sm font-medium', box.name.trim().length === 0 && 'text-muted-foreground')}>
          {box.name.trim().length === 0 ? 'Unnamed rule' : box.name}
        </p>
        {state?.changed && (
          <span
            role='img'
            aria-label='Changed since published'
            title='Changed since published'
            className='size-2 shrink-0 rounded-full bg-primary'
          />
        )}
        {issues > 0 && (
          <span
            title={state?.issues.join('\n')}
            className='inline-flex shrink-0 items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/15 px-1.5 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300'
          >
            <AlertTriangle className='size-3' />
            {issues}
          </span>
        )}
      </div>
      <div className='mt-2 flex flex-wrap items-center gap-1.5'>
        <Chip icon={<Bot className='size-3 shrink-0' />}>{agentsChip(box)}</Chip>
        {condition && <Chip icon={<Clock className='size-3 shrink-0' />}>{condition}</Chip>}
        {!box.enabled && <Chip>Disabled</Chip>}
      </div>
      <Handle type='source' position={Position.Right} className={HANDLE} isConnectable={!readOnly} />
    </div>
  )
}

export function HeaderNode({ data }: NodeProps<HeaderNodeType>) {
  return (
    <div style={{ width: 240 }} className='px-1'>
      <p className='text-xs font-semibold uppercase tracking-wide text-foreground'>{data.title}</p>
      <p className='whitespace-nowrap text-xs text-muted-foreground'>{data.hint}</p>
    </div>
  )
}
