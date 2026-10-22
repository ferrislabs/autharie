import type { Tone } from '@/components/ui/status-badge'
import type { AgentDecision, AgentPolicy, DecisionQuery } from './types'

export const DECISIONS_LIMIT = 100
export const DECISIONS_REFRESH_MS = 5000

export type DecisionRange = '1h' | '24h' | '7d'

export interface DecisionFilters {
  decision: 'all' | 'allow' | 'deny'
  tool: string
  subject: string
  range: DecisionRange
}

export const RANGES: { id: DecisionRange; label: string; ms: number }[] = [
  { id: '1h', label: 'Last hour', ms: 3_600_000 },
  { id: '24h', label: 'Last 24 hours', ms: 86_400_000 },
  { id: '7d', label: 'Last 7 days', ms: 604_800_000 },
]

export function emptyFilters(): DecisionFilters {
  return { decision: 'all', tool: '', subject: '', range: '24h' }
}

export function hasFilters(filters: DecisionFilters): boolean {
  return filters.decision !== 'all' || filters.tool.trim() !== '' || filters.subject.trim() !== ''
}

export function buildDecisionQuery(filters: DecisionFilters, now: Date): DecisionQuery {
  const span = RANGES.find((range) => range.id === filters.range)?.ms ?? RANGES[1].ms
  const query: DecisionQuery = {
    from: new Date(now.getTime() - span).toISOString(),
    to: now.toISOString(),
    limit: DECISIONS_LIMIT,
  }
  if (filters.decision !== 'all') query.decision = filters.decision
  const tool = filters.tool.trim()
  if (tool !== '') query.tool = tool
  const subject = filters.subject.trim()
  if (subject !== '') query.subject = subject
  return query
}

export function isAllow(decision: AgentDecision['decision']): boolean {
  if (typeof decision === 'boolean') return decision
  return decision.toLowerCase() === 'allow'
}

export function decisionBadge(decision: AgentDecision['decision']): { tone: Tone; label: string } {
  return isAllow(decision) ? { tone: 'success', label: 'Allow' } : { tone: 'danger', label: 'Deny' }
}

export function subjectLabel(decision: AgentDecision): string {
  return `${decision.subject_type} ${decision.subject_id}`
}

// FerrisKey knows a policy written through Autharie as `autharie:<policy id>`.
const MANAGED_PREFIX = 'autharie:'

export function policyLabel(name: string, policies: AgentPolicy[]): string {
  if (!name.startsWith(MANAGED_PREFIX)) return name
  const id = name.slice(MANAGED_PREFIX.length)

  return policies.find((policy) => policy.id === id)?.name ?? name
}

// A tool list is one request decided tool by tool, so the request id repeats.
export function decisionKey(decision: AgentDecision, index: number): string {
  return `${decision.request_id}:${decision.tool ?? decision.method}:${index}`
}
