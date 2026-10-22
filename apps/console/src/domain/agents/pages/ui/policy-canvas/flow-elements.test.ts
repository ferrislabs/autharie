import { describe, expect, it } from 'vitest'
import { docFromPolicies, moveBoxes, updateBox, type RuleBox } from '../../../canvas-model'
import { compileRule } from '../../../compile-rule'
import type { AgentPolicy, RuleModel } from '../../../types'
import { canvasHeight, mergeEdges, mergeNodes, type CanvasNode } from './flow-elements'

const model: RuleModel = {
  version: 1,
  effect: 'permit',
  who: { kind: 'roles', roles: ['admin'] },
  agents: { kind: 'any' },
  what: { kind: 'read-only', server: null },
  when: null,
}

function policy(id: string, over: Partial<AgentPolicy> = {}): AgentPolicy {
  const compiled = compileRule(model)
  return {
    id,
    name: id,
    enabled: true,
    rule: model,
    cedar: compiled.ok ? compiled.cedar : '',
    created_at: '',
    updated_at: '',
    ...over,
  }
}

describe('mergeNodes', () => {
  it('adds the three column headers and one node per box', () => {
    const nodes = mergeNodes(docFromPolicies([policy('a')]), [])
    expect(nodes.filter((node) => node.type === 'header').map((node) => node.id)).toEqual([
      'header:who',
      'header:rule',
      'header:what',
    ])
    expect(nodes.map((node) => node.id)).toEqual(
      expect.arrayContaining(['rule:a', 'who:role:admin', 'what:read-only:*']),
    )
  })

  it('keeps nodes, and their measurements, when nothing changed', () => {
    const doc = docFromPolicies([policy('a')])
    const first = mergeNodes(doc, []).map((node) => ({ ...node, measured: { width: 10, height: 10 } })) as CanvasNode[]
    const second = mergeNodes(doc, first)
    first.forEach((node, index) => expect(second[index]).toBe(node))
  })

  it('keeps the measurement of a node whose box changed', () => {
    const doc = docFromPolicies([policy('a')])
    const first = mergeNodes(doc, []).map((node) => ({ ...node, measured: { width: 10, height: 10 }, selected: true })) as CanvasNode[]
    const box = doc.boxes.find((entry) => entry.id === 'rule:a') as RuleBox
    const second = mergeNodes(updateBox(doc, { ...box, name: 'x' }), first)
    const node = second.find((entry) => entry.id === 'rule:a')
    expect(node?.measured).toEqual({ width: 10, height: 10 })
    expect(node?.selected).toBe(true)
  })

  it('follows the document positions except while dragging', () => {
    const doc = docFromPolicies([policy('a')])
    const first = mergeNodes(doc, [])
    const moved = moveBoxes(doc, { 'rule:a': { x: 7, y: 8 } })
    expect(mergeNodes(moved, first).find((node) => node.id === 'rule:a')?.position).toEqual({ x: 7, y: 8 })
    const dragging = first.map((node) => (node.id === 'rule:a' ? { ...node, dragging: true, position: { x: 1, y: 2 } } : node)) as CanvasNode[]
    expect(mergeNodes(moved, dragging).find((node) => node.id === 'rule:a')?.position).toEqual({ x: 1, y: 2 })
  })

  it('drops nodes of removed boxes', () => {
    const first = mergeNodes(docFromPolicies([policy('a'), policy('b')]), [])
    const next = mergeNodes(docFromPolicies([policy('a')]), first)
    expect(next.some((node) => node.id === 'rule:b')).toBe(false)
  })
})

describe('mergeEdges', () => {
  it('colours links after the effect of their rule', () => {
    const doc = docFromPolicies([policy('a', { rule: { ...model, effect: 'forbid' } }), policy('b', { enabled: false })])
    const edges = mergeEdges(doc, [])
    expect(edges).toHaveLength(4)
    expect(edges.filter((edge) => edge.data?.ruleId === 'rule:a').every((edge) => edge.data?.effect === 'forbid')).toBe(true)
    expect(edges.find((edge) => edge.data?.ruleId === 'rule:b')?.style?.strokeDasharray).toBe('6 5')
  })

  it('keeps unchanged edges and their selection', () => {
    const doc = docFromPolicies([policy('a')])
    const first = mergeEdges(doc, []).map((edge) => ({ ...edge, selected: true }))
    const second = mergeEdges(doc, first)
    second.forEach((edge, index) => expect(edge).toBe(first[index]))
  })
})

describe('canvasHeight', () => {
  it('stays within bounds', () => {
    expect(canvasHeight(docFromPolicies([]))).toBe(320)
    const many = Array.from({ length: 12 }, (_, index) => policy(`p${index}`))
    expect(canvasHeight(docFromPolicies(many))).toBe(720)
  })
})
