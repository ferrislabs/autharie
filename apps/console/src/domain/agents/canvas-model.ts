import { compileRule } from './compile-rule'
import type { PolicyTemplate } from './policy-templates'
import type { Catalogue } from './tool-catalogue'
import type {
  AgentPolicy,
  AgentPolicyInput,
  Effect,
  RuleModel,
  What,
  When,
  WhichAgents,
  Who,
} from './types'

export type Subject =
  | { type: 'any' }
  | { type: 'group' | 'role' | 'user' | 'agent'; value: string }

export type SubjectType = Subject['type']

export type Target =
  | { type: 'read-only'; server: string | null }
  | { type: 'server'; server: string }
  | { type: 'tools'; tools: string[] }

export interface Point {
  x: number
  y: number
}

export interface WhoBox {
  id: string
  kind: 'who'
  subject: Subject
}

export interface RuleBox {
  id: string
  kind: 'rule'
  policyId: string | null
  name: string
  description: string
  enabled: boolean
  effect: Effect
  agents: WhichAgents
  when: When | null
}

export interface WhatBox {
  id: string
  kind: 'what'
  target: Target
}

export type Box = WhoBox | RuleBox | WhatBox
export type BoxKind = Box['kind']

export interface Link {
  id: string
  source: string
  target: string
}

export interface CanvasDoc {
  boxes: Box[]
  links: Link[]
  positions: Record<string, Point>
}

export type RuleOutcome = { ok: true; rule: RuleModel } | { ok: false; issues: string[] }

export interface CanvasDiff {
  create: AgentPolicyInput[]
  createIds: string[]
  update: { id: string; input: AgentPolicyInput }[]
  delete: string[]
}

export interface BlockingRule {
  ruleId: string
  name: string
  issues: string[]
}

export interface Pending {
  diff: CanvasDiff
  count: number
  changedRuleIds: Set<string>
  blocking: BlockingRule[]
}

export const COLUMN_X: Record<BoxKind, number> = { who: 0, rule: 420, what: 860 }
export const COLUMN_WIDTH: Record<BoxKind, number> = { who: 240, rule: 300, what: 240 }
export const RULE_ROW = 150
export const SIDE_ROW = 88
const RULE_HEIGHT = 120
const SIDE_HEIGHT = 64

export const MIXED_SUBJECTS =
  'A rule applies to groups, roles, users or agents — not a mix. Split it into two rules.'

export function emptyDoc(): CanvasDoc {
  return { boxes: [], links: [], positions: {} }
}

export function subjectKey(subject: Subject): string {
  return subject.type === 'any' ? 'any' : `${subject.type}:${subject.value}`
}

export function targetKey(target: Target): string {
  switch (target.type) {
    case 'read-only':
      return `read-only:${target.server ?? '*'}`
    case 'server':
      return `server:${target.server}`
    case 'tools':
      return `tools:${[...target.tools].sort().join(',')}`
  }
}

export function subjectsOf(who: Who): Subject[] {
  switch (who.kind) {
    case 'any-user':
      return [{ type: 'any' }]
    case 'groups':
      return who.groups.map((value) => ({ type: 'group', value }))
    case 'roles':
      return who.roles.map((value) => ({ type: 'role', value }))
    case 'users':
      return who.users.map((value) => ({ type: 'user', value }))
    case 'agents':
      return who.clientIds.map((value) => ({ type: 'agent', value }))
  }
}

export function targetOf(what: What): Target {
  switch (what.kind) {
    case 'read-only':
      return { type: 'read-only', server: what.server }
    case 'server':
      return { type: 'server', server: what.server }
    case 'tools':
      return { type: 'tools', tools: [...what.tools] }
  }
}

function linkId(source: string, target: string): string {
  return `${source}->${target}`
}

function unique(values: string[]): string[] {
  return [...new Set(values)]
}

function ruleBoxOf(id: string, policyId: string | null, rule: RuleModel, name: string, description: string, enabled: boolean): RuleBox {
  return {
    id,
    kind: 'rule',
    policyId,
    name,
    description,
    enabled,
    effect: rule.effect,
    agents: rule.agents,
    when: rule.when,
  }
}

function column(doc: CanvasDoc, kind: BoxKind): Box[] {
  return doc.boxes.filter((box) => box.kind === kind)
}

export function layoutDoc(doc: CanvasDoc): Record<string, Point> {
  const positions: Record<string, Point> = {}
  const rules = column(doc, 'rule')
  rules.forEach((box, index) => {
    positions[box.id] = { x: COLUMN_X.rule, y: index * RULE_ROW }
  })

  for (const kind of ['who', 'what'] as const) {
    const entries = column(doc, kind).map((box, order) => {
      const rows = doc.links
        .filter((link) => (kind === 'who' ? link.source === box.id : link.target === box.id))
        .map((link) => positions[kind === 'who' ? link.target : link.source])
        .filter((point): point is Point => point !== undefined)
      const desired =
        rows.length === 0
          ? Number.POSITIVE_INFINITY
          : rows.reduce((sum, point) => sum + point.y, 0) / rows.length + (RULE_HEIGHT - SIDE_HEIGHT) / 2
      return { box, order, desired }
    })
    entries.sort((a, b) => (a.desired === b.desired ? a.order - b.order : a.desired - b.desired))
    let previous = Number.NEGATIVE_INFINITY
    for (const entry of entries) {
      const floor = previous === Number.NEGATIVE_INFINITY ? 0 : previous + SIDE_ROW
      const y = Number.isFinite(entry.desired) ? Math.max(entry.desired, floor) : floor
      positions[entry.box.id] = { x: COLUMN_X[kind], y }
      previous = y
    }
  }
  return positions
}

export function docFromPolicies(policies: AgentPolicy[]): CanvasDoc {
  const boxes: Box[] = []
  const links: Link[] = []
  const known = new Set<string>()

  function remember(box: Box) {
    if (known.has(box.id)) return
    known.add(box.id)
    boxes.push(box)
  }

  function connect(source: string, target: string) {
    const id = linkId(source, target)
    if (links.some((link) => link.id === id)) return
    links.push({ id, source, target })
  }

  for (const policy of policies) {
    const rule = policy.rule
    if (!rule) continue
    const box = ruleBoxOf(`rule:${policy.id}`, policy.id, rule, policy.name, policy.description ?? '', policy.enabled)
    remember(box)
    for (const subject of subjectsOf(rule.who)) {
      const who: WhoBox = { id: `who:${subjectKey(subject)}`, kind: 'who', subject }
      remember(who)
      connect(who.id, box.id)
    }
    const target = targetOf(rule.what)
    const what: WhatBox = { id: `what:${targetKey(target)}`, kind: 'what', target }
    remember(what)
    connect(box.id, what.id)
  }

  const doc: CanvasDoc = { boxes, links, positions: {} }
  return { ...doc, positions: layoutDoc(doc) }
}

export function keepPositions(next: CanvasDoc, previous: CanvasDoc): CanvasDoc {
  const positions = { ...next.positions }
  for (const box of next.boxes) {
    const kept = previous.positions[box.id]
    if (kept) positions[box.id] = kept
  }
  return { ...next, positions }
}

export function sameDocShape(a: CanvasDoc, b: CanvasDoc): boolean {
  return JSON.stringify([a.boxes, a.links]) === JSON.stringify([b.boxes, b.links])
}

function boxById(doc: CanvasDoc, id: string): Box | undefined {
  return doc.boxes.find((box) => box.id === id)
}

function whoFrom(subjects: Subject[]): { who: Who } | { issue: string } {
  const types = unique(subjects.map((subject) => subject.type))
  if (types.includes('any')) {
    if (types.length > 1) {
      return { issue: '“Any user” cannot be combined with other people. Remove one of them, or split it into two rules.' }
    }
    return { who: { kind: 'any-user' } }
  }
  if (types.length > 1) return { issue: MIXED_SUBJECTS }
  const values = unique(subjects.flatMap((subject) => (subject.type === 'any' ? [] : [subject.value])))
  switch (types[0]) {
    case 'group':
      return { who: { kind: 'groups', groups: values } }
    case 'role':
      return { who: { kind: 'roles', roles: values } }
    case 'user':
      return { who: { kind: 'users', users: values } }
    default:
      return { who: { kind: 'agents', clientIds: values } }
  }
}

function whatFrom(targets: WhatBox[]): { what: What } | { issue: string } {
  const tools = targets.filter((box) => box.target.type === 'tools')
  if (tools.length === targets.length) {
    const merged = unique(
      targets.flatMap((box) => (box.target.type === 'tools' ? box.target.tools : [])),
    )
    return { what: { kind: 'tools', tools: merged } }
  }
  if (targets.length > 1) {
    return {
      issue:
        'Read-only and whole-server targets cannot be combined with other targets. Keep one of them, or split it into two rules.',
    }
  }
  const target = targets[0].target
  if (target.type === 'read-only') return { what: { kind: 'read-only', server: target.server } }
  if (target.type === 'server') return { what: { kind: 'server', server: target.server } }
  return { what: { kind: 'tools', tools: [...target.tools] } }
}

export function ruleOf(doc: CanvasDoc, ruleId: string): RuleOutcome {
  const box = boxById(doc, ruleId)
  if (!box || box.kind !== 'rule') return { ok: false, issues: ['This rule no longer exists.'] }

  const issues: string[] = []
  if (box.name.trim().length === 0) issues.push('Give the rule a name.')

  const subjects = doc.links
    .filter((link) => link.target === ruleId)
    .map((link) => boxById(doc, link.source))
    .filter((entry): entry is WhoBox => entry?.kind === 'who')
    .map((entry) => entry.subject)
  const targets = doc.links
    .filter((link) => link.source === ruleId)
    .map((link) => boxById(doc, link.target))
    .filter((entry): entry is WhatBox => entry?.kind === 'what')

  let who: Who | null = null
  if (subjects.length === 0) {
    issues.push('Link at least one person to this rule: drag from a Who box onto it.')
  } else {
    const built = whoFrom(subjects)
    if ('issue' in built) issues.push(built.issue)
    else who = built.who
  }

  let what: What | null = null
  if (targets.length === 0) {
    issues.push('Link at least one tool target: drag from this rule onto a What box.')
  } else {
    const built = whatFrom(targets)
    if ('issue' in built) issues.push(built.issue)
    else what = built.what
  }

  if (issues.length > 0 || who === null || what === null) return { ok: false, issues }

  const rule: RuleModel = {
    version: 1,
    effect: box.effect,
    who,
    agents: box.agents,
    what,
    when: box.when,
  }
  const compiled = compileRule(rule)
  return compiled.ok ? { ok: true, rule } : { ok: false, issues: compiled.issues }
}

function sorted(values: string[]): string[] {
  return unique(values).sort()
}

function canonicalWho(who: Who): unknown {
  switch (who.kind) {
    case 'any-user':
      return { kind: who.kind }
    case 'groups':
      return { kind: who.kind, values: sorted(who.groups) }
    case 'roles':
      return { kind: who.kind, values: sorted(who.roles) }
    case 'users':
      return { kind: who.kind, values: sorted(who.users) }
    case 'agents':
      return { kind: who.kind, values: sorted(who.clientIds) }
  }
}

function canonicalWhat(what: What): unknown {
  switch (what.kind) {
    case 'tools':
      return { kind: what.kind, tools: sorted(what.tools) }
    default:
      return what
  }
}

function canonical(rule: RuleModel): string {
  return JSON.stringify({
    effect: rule.effect,
    who: canonicalWho(rule.who),
    agents:
      rule.agents.kind === 'any'
        ? { kind: 'any' }
        : { kind: 'clients', clientIds: sorted(rule.agents.clientIds) },
    what: canonicalWhat(rule.what),
    when:
      rule.when === null
        ? null
        : {
            weekdays: [...new Set(rule.when.weekdays)].sort((a, b) => a - b),
            hours: rule.when.hours,
          },
  })
}

export function sameRule(a: RuleModel, b: RuleModel): boolean {
  return canonical(a) === canonical(b)
}

function inputOf(box: RuleBox, rule: RuleModel): AgentPolicyInput {
  const compiled = compileRule(rule)
  const description = box.description.trim()
  return {
    name: box.name.trim(),
    enabled: box.enabled,
    rule,
    cedar: compiled.ok ? compiled.cedar : '',
    ...(description.length > 0 ? { description } : {}),
  }
}

function metaChanged(original: AgentPolicy, box: RuleBox): boolean {
  return (
    original.name !== box.name.trim() ||
    (original.description ?? '').trim() !== box.description.trim() ||
    original.enabled !== box.enabled
  )
}

export function changeCount(diff: CanvasDiff): number {
  return diff.create.length + diff.update.length + diff.delete.length
}

export function pendingChanges(policies: AgentPolicy[], doc: CanvasDoc): Pending {
  const diff: CanvasDiff = { create: [], createIds: [], update: [], delete: [] }
  const changedRuleIds = new Set<string>()
  const blocking: BlockingRule[] = []
  const claimed = new Set<string>()

  for (const box of column(doc, 'rule') as RuleBox[]) {
    const outcome = ruleOf(doc, box.id)
    const original = box.policyId === null ? undefined : policies.find((policy) => policy.id === box.policyId)
    if (box.policyId !== null) claimed.add(box.policyId)
    if (box.policyId !== null && !original) continue

    if (!original) {
      changedRuleIds.add(box.id)
      if (outcome.ok) {
        diff.create.push(inputOf(box, outcome.rule))
        diff.createIds.push(box.id)
      } else {
        blocking.push({ ruleId: box.id, name: box.name, issues: outcome.issues })
      }
      continue
    }

    if (outcome.ok) {
      const changed = !original.rule || metaChanged(original, box) || !sameRule(original.rule, outcome.rule)
      if (changed) {
        changedRuleIds.add(box.id)
        diff.update.push({ id: original.id, input: inputOf(box, outcome.rule) })
      }
      continue
    }

    const originalOutcome = original.rule ? compileRule(original.rule) : null
    const untouched = originalOutcome !== null && !originalOutcome.ok && !metaChanged(original, box)
    if (!untouched) {
      changedRuleIds.add(box.id)
      blocking.push({ ruleId: box.id, name: box.name, issues: outcome.issues })
    }
  }

  for (const policy of policies) {
    if (policy.rule && !claimed.has(policy.id)) diff.delete.push(policy.id)
  }

  return { diff, count: changeCount(diff) + blocking.length, changedRuleIds, blocking }
}

export function diffDoc(policies: AgentPolicy[], doc: CanvasDoc): CanvasDiff {
  return pendingChanges(policies, doc).diff
}

export function nextId(doc: CanvasDoc, prefix: BoxKind): string {
  const taken = new Set(doc.boxes.map((box) => box.id))
  for (let n = 1; ; n += 1) {
    const id = `${prefix}:new-${n}`
    if (!taken.has(id)) return id
  }
}

export function newWhoBox(doc: CanvasDoc, subject: Subject): WhoBox {
  return { id: nextId(doc, 'who'), kind: 'who', subject }
}

export function newWhatBox(doc: CanvasDoc, target: Target): WhatBox {
  return { id: nextId(doc, 'what'), kind: 'what', target }
}

export function newRuleBox(doc: CanvasDoc): RuleBox {
  return {
    id: nextId(doc, 'rule'),
    kind: 'rule',
    policyId: null,
    name: 'New rule',
    description: '',
    enabled: true,
    effect: 'permit',
    agents: { kind: 'any' },
    when: null,
  }
}

export function addBox(doc: CanvasDoc, box: Box, position?: Point): CanvasDoc {
  if (doc.boxes.some((entry) => entry.id === box.id)) return doc
  const at = position ?? { x: COLUMN_X[box.kind], y: lowestY(doc, box.kind) }
  return { ...doc, boxes: [...doc.boxes, box], positions: { ...doc.positions, [box.id]: at } }
}

function lowestY(doc: CanvasDoc, kind: BoxKind): number {
  const rows = column(doc, kind)
    .map((box) => doc.positions[box.id]?.y)
    .filter((y): y is number => y !== undefined)
  const step = kind === 'rule' ? RULE_ROW : SIDE_ROW
  return rows.length === 0 ? 0 : Math.max(...rows) + step
}

export function updateBox(doc: CanvasDoc, next: Box): CanvasDoc {
  const current = boxById(doc, next.id)
  if (!current || current.kind !== next.kind) return doc
  return { ...doc, boxes: doc.boxes.map((box) => (box.id === next.id ? next : box)) }
}

export function removeBox(doc: CanvasDoc, id: string): CanvasDoc {
  const positions = { ...doc.positions }
  delete positions[id]
  return {
    boxes: doc.boxes.filter((box) => box.id !== id),
    links: doc.links.filter((link) => link.source !== id && link.target !== id),
    positions,
  }
}

export function moveBoxes(doc: CanvasDoc, moves: Record<string, Point>): CanvasDoc {
  const positions = { ...doc.positions }
  for (const [id, point] of Object.entries(moves)) {
    if (boxById(doc, id)) positions[id] = point
  }
  return { ...doc, positions }
}

export function canLink(doc: CanvasDoc, source: string, target: string): boolean {
  const from = boxById(doc, source)
  const to = boxById(doc, target)
  if (!from || !to) return false
  const valid = (from.kind === 'who' && to.kind === 'rule') || (from.kind === 'rule' && to.kind === 'what')
  if (!valid) return false
  return !doc.links.some((link) => link.source === source && link.target === target)
}

export function addLink(doc: CanvasDoc, source: string, target: string): CanvasDoc {
  if (!canLink(doc, source, target)) return doc
  return { ...doc, links: [...doc.links, { id: linkId(source, target), source, target }] }
}

export function removeLink(doc: CanvasDoc, id: string): CanvasDoc {
  return { ...doc, links: doc.links.filter((link) => link.id !== id) }
}

export function duplicateRule(doc: CanvasDoc, ruleId: string): CanvasDoc {
  const original = boxById(doc, ruleId)
  if (!original || original.kind !== 'rule') return doc
  const copy: RuleBox = {
    ...original,
    id: nextId(doc, 'rule'),
    policyId: null,
    name: `${original.name} copy`,
  }
  const from = doc.positions[ruleId] ?? { x: COLUMN_X.rule, y: 0 }
  let next = addBox(doc, copy, { x: from.x, y: from.y + RULE_ROW - 10 })
  for (const link of doc.links) {
    if (link.target === ruleId) next = addLink(next, link.source, copy.id)
    if (link.source === ruleId) next = addLink(next, copy.id, link.target)
  }
  return next
}

export interface DroppedTemplate {
  boxes: Box[]
  links: Link[]
  ruleId: string
}

export function ruleFromTemplate(
  template: PolicyTemplate,
  catalogue: Catalogue,
  doc: CanvasDoc = emptyDoc(),
): DroppedTemplate {
  const draft = template.build(catalogue)
  const rule = newRuleBox(doc)
  const box: RuleBox = {
    ...ruleBoxOf(rule.id, null, draft.rule, draft.name, draft.description, true),
  }
  let scratch = addBox(doc, box)
  const boxes: Box[] = [box]
  const links: Link[] = []
  for (const subject of subjectsOf(draft.rule.who)) {
    const who = newWhoBox(scratch, subject)
    scratch = addBox(scratch, who)
    boxes.push(who)
    links.push({ id: linkId(who.id, box.id), source: who.id, target: box.id })
  }
  const what = newWhatBox(scratch, targetOf(draft.rule.what))
  boxes.push(what)
  links.push({ id: linkId(box.id, what.id), source: box.id, target: what.id })
  return { boxes, links, ruleId: box.id }
}

export function addTemplate(
  doc: CanvasDoc,
  template: PolicyTemplate,
  at: Point,
  catalogue: Catalogue,
): CanvasDoc {
  const dropped = ruleFromTemplate(template, catalogue, doc)
  let whoRow = 0
  const positions = { ...doc.positions }
  for (const box of dropped.boxes) {
    if (box.kind === 'rule') positions[box.id] = { x: COLUMN_X.rule, y: at.y }
    else if (box.kind === 'who') {
      positions[box.id] = { x: COLUMN_X.who, y: at.y + (RULE_HEIGHT - SIDE_HEIGHT) / 2 + whoRow * SIDE_ROW }
      whoRow += 1
    } else positions[box.id] = { x: COLUMN_X.what, y: at.y + (RULE_HEIGHT - SIDE_HEIGHT) / 2 }
  }
  return {
    boxes: [...doc.boxes, ...dropped.boxes],
    links: [...doc.links, ...dropped.links],
    positions,
  }
}

export function connectedRules(doc: CanvasDoc, boxId: string): string[] {
  const box = boxById(doc, boxId)
  if (!box) return []
  if (box.kind === 'rule') return [box.id]
  const ids = new Set<string>()
  for (const link of doc.links) {
    if (link.source === boxId) ids.add(link.target)
    if (link.target === boxId) ids.add(link.source)
  }
  return [...ids]
}

export interface Focus {
  rules: Set<string>
  boxes: Set<string>
}

export function focusOf(doc: CanvasDoc, boxId: string): Focus {
  const rules = new Set(connectedRules(doc, boxId))
  const boxes = new Set<string>([boxId])
  for (const link of doc.links) {
    if (rules.has(link.source) || rules.has(link.target)) {
      boxes.add(link.source)
      boxes.add(link.target)
    }
  }
  return { rules, boxes }
}

export function usedBy(doc: CanvasDoc, boxId: string): number {
  return doc.links.filter((link) => link.source === boxId || link.target === boxId).length
}
