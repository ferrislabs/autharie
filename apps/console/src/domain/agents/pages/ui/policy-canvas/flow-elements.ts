import { MarkerType, type Edge, type Node } from '@xyflow/react'
import {
  COLUMN_X,
  type BoxKind,
  type CanvasDoc,
  type RuleBox,
  type WhatBox,
  type WhoBox,
} from '../../../canvas-model'
import type { Effect } from '../../../types'

export const PERMIT = '#10b981'
export const FORBID = 'var(--destructive)'
export const MUTED = 'var(--muted-foreground)'
export const HEADER_Y = -84

export type WhoNodeType = Node<{ box: WhoBox }, 'who'>
export type RuleNodeType = Node<{ box: RuleBox }, 'rule'>
export type WhatNodeType = Node<{ box: WhatBox }, 'what'>
export type HeaderNodeType = Node<{ title: string; hint: string }, 'header'>
export type CanvasNode = WhoNodeType | RuleNodeType | WhatNodeType | HeaderNodeType

export interface LinkData extends Record<string, unknown> {
  ruleId: string
  effect: Effect
  enabled: boolean
}

export type CanvasEdge = Edge<LinkData, 'link'>

const HEADERS: Record<BoxKind, { title: string; hint: string }> = {
  who: { title: 'Who', hint: 'People asking' },
  rule: { title: 'Rule', hint: 'Allow or deny, through which agent, when' },
  what: { title: 'What', hint: 'Tools they may call' },
}

const COLUMNS: BoxKind[] = ['who', 'rule', 'what']

export function strokeOf(effect: Effect, enabled: boolean): string {
  if (!enabled) return MUTED
  return effect === 'permit' ? PERMIT : FORBID
}

function headerNodes(previous: CanvasNode[]): CanvasNode[] {
  return COLUMNS.map((kind) => {
    const id = `header:${kind}`
    const existing = previous.find((node) => node.id === id)
    if (existing) return existing
    return {
      id,
      type: 'header',
      position: { x: COLUMN_X[kind], y: HEADER_Y },
      data: HEADERS[kind],
      draggable: false,
      selectable: false,
      focusable: false,
      connectable: false,
    } satisfies HeaderNodeType
  })
}

export function mergeNodes(doc: CanvasDoc, previous: CanvasNode[]): CanvasNode[] {
  const known = new Map(previous.map((node) => [node.id, node]))
  const boxes = doc.boxes.map((box): CanvasNode => {
    const old = known.get(box.id)
    const docPosition = doc.positions[box.id] ?? { x: COLUMN_X[box.kind], y: 0 }
    const position = old?.dragging ? old.position : docPosition
    if (
      old &&
      old.type === box.kind &&
      (old.data as { box: unknown }).box === box &&
      old.position.x === position.x &&
      old.position.y === position.y
    ) {
      return old
    }
    return { ...old, id: box.id, type: box.kind, position, data: { box } } as CanvasNode
  })
  return [...headerNodes(previous), ...boxes]
}

export function mergeEdges(doc: CanvasDoc, previous: CanvasEdge[]): CanvasEdge[] {
  const known = new Map(previous.map((edge) => [edge.id, edge]))
  const rules = new Map(
    doc.boxes.filter((box): box is RuleBox => box.kind === 'rule').map((box) => [box.id, box]),
  )
  const edges: CanvasEdge[] = []
  for (const link of doc.links) {
    const rule = rules.get(link.target) ?? rules.get(link.source)
    if (!rule) continue
    const old = known.get(link.id)
    if (old && old.data?.ruleId === rule.id && old.data.effect === rule.effect && old.data.enabled === rule.enabled) {
      edges.push(old)
      continue
    }
    const color = strokeOf(rule.effect, rule.enabled)
    edges.push({
      ...old,
      id: link.id,
      type: 'link',
      source: link.source,
      target: link.target,
      data: { ruleId: rule.id, effect: rule.effect, enabled: rule.enabled },
      markerEnd: { type: MarkerType.ArrowClosed, color, width: 6, height: 6 },
      style: {
        stroke: color,
        strokeWidth: rule.enabled ? 2.5 : 2,
        strokeDasharray: rule.enabled ? undefined : '6 5',
      },
    })
  }
  return edges
}

export function canvasHeight(doc: CanvasDoc): number {
  const bottom = Math.max(0, ...Object.values(doc.positions).map((point) => point.y))
  return Math.min(720, Math.max(320, Math.round(bottom + 260)))
}
