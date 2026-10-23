import { useStore, useViewport } from '@xyflow/react'
import { ruleOf, usedBy, COLUMN_WIDTH, type Box, type CanvasDoc } from '../../../canvas-model'
import { compileRule } from '../../../compile-rule'
import type { AgentPolicy } from '../../../types'
import { RuleEditor } from './rule-editor'
import { WhatEditor } from './what-editor'
import { WhoEditor } from './who-editor'
import { useCanvasUi } from './canvas-context'

const PANEL_WIDTH = 340
const GAP = 14
const MIN_ROOM = 280

export function EditorLayer({
  doc,
  editingId,
  policies,
  onChange,
  onClose,
  onOpenAdvanced,
}: {
  doc: CanvasDoc
  editingId: string | null
  policies: AgentPolicy[]
  onChange: (box: Box) => void
  onClose: () => void
  onOpenAdvanced: (policy: AgentPolicy) => void
}) {
  const { readOnly, status } = useCanvasUi()
  const viewport = useViewport()
  const width = useStore((state) => state.width)
  const height = useStore((state) => state.height)

  const box = editingId === null ? undefined : doc.boxes.find((entry) => entry.id === editingId)
  const origin = box ? doc.positions[box.id] : undefined
  if (!box || !origin) return null

  const nodeWidth = COLUMN_WIDTH[box.kind] * viewport.zoom
  const right = origin.x * viewport.zoom + viewport.x + nodeWidth + GAP
  const left = origin.x * viewport.zoom + viewport.x - PANEL_WIDTH - GAP
  const x = right + PANEL_WIDTH + 8 <= width ? right : left >= 8 ? left : Math.max(8, width - PANEL_WIDTH - 8)
  const y = Math.min(Math.max(8, origin.y * viewport.zoom + viewport.y - 8), Math.max(8, height - MIN_ROOM))
  const usage = usedBy(doc, box.id)

  const policy =
    box.kind === 'rule' && box.policyId !== null
      ? policies.find((entry) => entry.id === box.policyId)
      : undefined
  const outcome = box.kind === 'rule' ? ruleOf(doc, box.id) : null
  const compiled = outcome?.ok ? compileRule(outcome.rule) : null

  return (
    <div
      role='dialog'
      aria-label='Edit box'
      className='nowheel nopan nodrag absolute z-30 flex flex-col rounded-lg border bg-popover text-popover-foreground shadow-lg'
      style={{ left: x, top: y, width: PANEL_WIDTH, maxHeight: Math.max(MIN_ROOM, height - y - 8) }}
      onContextMenu={(event) => event.stopPropagation()}
    >
      {box.kind === 'who' && (
        <WhoEditor key={box.id} box={box} usage={usage} readOnly={readOnly} onChange={onChange} onClose={onClose} />
      )}
      {box.kind === 'what' && (
        <WhatEditor key={box.id} box={box} usage={usage} readOnly={readOnly} onChange={onChange} onClose={onClose} />
      )}
      {box.kind === 'rule' && (
        <RuleEditor
          key={box.id}
          box={box}
          issues={status[box.id]?.issues ?? []}
          cedar={compiled?.ok ? compiled.cedar : null}
          readOnly={readOnly}
          canOpenAdvanced={policy !== undefined}
          onChange={onChange}
          onOpenAdvanced={() => policy && onOpenAdvanced(policy)}
          onClose={onClose}
        />
      )}
    </div>
  )
}
