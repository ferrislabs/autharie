import { summariseHours, summariseWeekdays } from './summary'
import { findTool, nameOfTool, SERVERS, serverOfTool } from './tool-catalogue'
import type { AgentPolicy, Effect, RuleModel, What, When, WhichAgents, Who } from './types'

export type Column = 'who' | 'agent' | 'what'

export const COLUMN_X: Record<Column, number> = { who: 0, agent: 460, what: 920 }
export const ROW_GAP = 84
export const GROUP_TOOLS_ABOVE = 3

export type AccessNodeData = {
  label: string
  caption: string
  any: boolean
  destructive: boolean
  members: string[]
}

export type AccessNode = {
  id: string
  type: Column
  position: { x: number; y: number }
  data: AccessNodeData
}

export type AccessEdgeData = {
  policyId: string
  effect: Effect
  enabled: boolean
  label: string | null
  tier: 1 | 2
}

export type AccessEdge = {
  id: string
  type: 'access'
  source: string
  target: string
  data: AccessEdgeData
}

export interface AccessGraph {
  nodes: AccessNode[]
  edges: AccessEdge[]
  unmapped: AgentPolicy[]
}

type Draft = Omit<AccessNode, 'position'>

function serverLabel(id: string): string {
  return SERVERS.find((server) => server.id === id)?.label ?? id
}

function draft(id: string, type: Column, label: string, caption: string, any = false, destructive = false): Draft {
  return { id, type, data: { label, caption, any, destructive, members: [] } }
}

function whoDrafts(who: Who): Draft[] {
  switch (who.kind) {
    case 'any-user':
      return [draft('who:any', 'who', 'Any user', 'everyone', true)]
    case 'groups':
      return who.groups.map((group) => draft(`who:group:${group}`, 'who', group, 'group'))
    case 'roles':
      return who.roles.map((role) => draft(`who:role:${role}`, 'who', role, 'role'))
    case 'users':
      return who.users.map((user) => draft(`who:user:${user}`, 'who', user, 'user'))
    case 'agents':
      return []
  }
}

function agentDrafts(agents: WhichAgents, autonomous: string[]): Draft[] {
  if (autonomous.length > 0) {
    return autonomous.map((id) => draft(`agent:client:${id}`, 'agent', id, 'acts on its own'))
  }
  if (agents.kind === 'any') return [draft('agent:any', 'agent', 'Any agent', 'any AI agent', true)]
  return agents.clientIds.map((id) => draft(`agent:client:${id}`, 'agent', id, 'agent'))
}

function whatDrafts(what: What): Draft[] {
  switch (what.kind) {
    case 'read-only':
      return [
        draft(
          `what:read-only:${what.server ?? 'all'}`,
          'what',
          'Read-only tools',
          what.server === null ? 'read-only, all servers' : `read-only, ${serverLabel(what.server)}`,
          what.server === null,
        ),
      ]
    case 'server':
      return [draft(`what:server:${what.server}`, 'what', serverLabel(what.server), 'whole server')]
    case 'tools':
      if (what.tools.length > GROUP_TOOLS_ABOVE) return [toolGroupDraft(what.tools)]
      return what.tools.map((tool) =>
        draft(
          `what:tool:${tool}`,
          'what',
          nameOfTool(tool),
          `tool, ${serverLabel(serverOfTool(tool))}`,
          false,
          findTool(tool)?.destructive ?? false,
        ),
      )
  }
}

function preview(names: string[]): string {
  const shown = names.slice(0, 2).join(', ')
  return names.length > 2 ? `${shown} +${names.length - 2}` : shown
}

function toolGroupDraft(tools: string[]): Draft {
  const sorted = [...tools].sort()
  const destructive = sorted.every((tool) => findTool(tool)?.destructive ?? false)
  const group = draft(
    `what:tools:${sorted.join(',')}`,
    'what',
    `${sorted.length} ${destructive ? 'destructive tools' : 'tools'}`,
    preview(sorted.map(nameOfTool)),
    false,
    destructive,
  )
  return { ...group, data: { ...group.data, members: sorted.map(nameOfTool) } }
}

export function conditionLabel(when: When | null): string | null {
  if (when === null) return null
  const parts: string[] = []
  if (when.weekdays.length > 0) parts.push(summariseWeekdays(when.weekdays))
  if (when.hours !== null) parts.push(summariseHours(when.hours))
  return parts.length > 0 ? parts.join(', ') : null
}

function edgeId(base: string, taken: Set<string>): string {
  let id = base
  for (let n = 2; taken.has(id); n += 1) id = `${base}#${n}`
  taken.add(id)
  return id
}

function layout(drafts: Map<string, Draft>): AccessNode[] {
  const nodes: AccessNode[] = []
  for (const column of Object.keys(COLUMN_X) as Column[]) {
    const rows = [...drafts.values()]
      .filter((entry) => entry.type === column)
      .sort((a, b) => {
        if (a.data.any !== b.data.any) return a.data.any ? -1 : 1
        return a.data.label.localeCompare(b.data.label) || a.id.localeCompare(b.id)
      })
    rows.forEach((entry, row) => {
      nodes.push({ ...entry, position: { x: COLUMN_X[column], y: row * ROW_GAP } })
    })
  }
  return nodes
}

export function buildAccessGraph(policies: AgentPolicy[]): AccessGraph {
  const drafts = new Map<string, Draft>()
  const edges: AccessEdge[] = []
  const unmapped: AgentPolicy[] = []

  function remember(list: Draft[]) {
    for (const entry of list) {
      if (!drafts.has(entry.id)) drafts.set(entry.id, entry)
    }
  }

  for (const policy of policies) {
    const rule = policy.rule
    if (!rule) {
      unmapped.push(policy)
      continue
    }
    const autonomous = rule.who.kind === 'agents' ? rule.who.clientIds : []
    const whos = whoDrafts(rule.who)
    const agents = agentDrafts(rule.agents, autonomous)
    const whats = whatDrafts(rule.what)
    remember(whos)
    remember(agents)
    remember(whats)

    const taken = new Set<string>()
    const base = { policyId: policy.id, effect: rule.effect, enabled: policy.enabled }
    for (const who of whos) {
      for (const agent of agents) {
        edges.push({
          id: edgeId(`${policy.id}:1`, taken),
          type: 'access',
          source: who.id,
          target: agent.id,
          data: { ...base, label: null, tier: 1 },
        })
      }
    }
    const label = conditionLabel(rule.when)
    for (const agent of agents) {
      for (const what of whats) {
        edges.push({
          id: edgeId(`${policy.id}:2`, taken),
          type: 'access',
          source: agent.id,
          target: what.id,
          data: { ...base, label, tier: 2 },
        })
      }
    }
  }

  return { nodes: layout(drafts), edges, unmapped }
}

export function connectedPolicies(graph: Pick<AccessGraph, 'edges'>, nodeId: string): string[] {
  const ids = new Set<string>()
  for (const edge of graph.edges) {
    if (edge.source === nodeId || edge.target === nodeId) ids.add(edge.data.policyId)
  }
  return [...ids]
}

function whoOfNode(id: string): Who | null {
  if (id === 'who:any') return { kind: 'any-user' }
  if (id.startsWith('who:group:')) return { kind: 'groups', groups: [id.slice('who:group:'.length)] }
  if (id.startsWith('who:role:')) return { kind: 'roles', roles: [id.slice('who:role:'.length)] }
  if (id.startsWith('who:user:')) return { kind: 'users', users: [id.slice('who:user:'.length)] }
  return null
}

function agentOfNode(id: string): WhichAgents | null {
  if (id === 'agent:any') return { kind: 'any' }
  if (id.startsWith('agent:client:')) return { kind: 'clients', clientIds: [id.slice('agent:client:'.length)] }
  return null
}

function whatOfNode(id: string): What | null {
  if (id === 'what:read-only:all') return { kind: 'read-only', server: null }
  if (id.startsWith('what:read-only:')) return { kind: 'read-only', server: id.slice('what:read-only:'.length) }
  if (id.startsWith('what:server:')) return { kind: 'server', server: id.slice('what:server:'.length) }
  if (id.startsWith('what:tool:')) return { kind: 'tools', tools: [id.slice('what:tool:'.length)] }
  if (id.startsWith('what:tools:')) return { kind: 'tools', tools: id.slice('what:tools:'.length).split(',') }
  return null
}

export function ruleFromConnection(sourceNodeId: string, targetNodeId: string): RuleModel | null {
  const what = whatOfNode(targetNodeId)
  if (what === null) return null

  const who = whoOfNode(sourceNodeId)
  if (who !== null) return { version: 1, effect: 'permit', who, agents: { kind: 'any' }, what, when: null }

  const agents = agentOfNode(sourceNodeId)
  if (agents !== null) {
    return { version: 1, effect: 'permit', who: { kind: 'any-user' }, agents, what, when: null }
  }

  return null
}
