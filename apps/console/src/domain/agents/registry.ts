import { formatDistanceStrict } from 'date-fns'
import type { Tone } from '@/components/ui/status-badge'
import type { AccessState, AgentClient, ClientSource } from './types'

export type RegistryFilter = 'all' | 'pending' | 'blocked'

export const ACCESS_BADGE: Record<AccessState, { tone: Tone; label: string }> = {
  approved: { tone: 'success', label: 'Approved' },
  pending: { tone: 'warning', label: 'Pending' },
  blocked: { tone: 'danger', label: 'Blocked' },
}

export const SOURCE_LABEL: Record<ClientSource, string> = {
  dynamic: 'Dynamic registration',
  metadata_document: 'Metadata document',
  autharie: 'Created here',
  seen: 'Seen in calls',
}

export function pendingCount(clients: AgentClient[]): number {
  return clients.filter((client) => client.access === 'pending').length
}

export function blockedCount(clients: AgentClient[]): number {
  return clients.filter((client) => client.access === 'blocked').length
}

export function filterClients(clients: AgentClient[], filter: RegistryFilter): AgentClient[] {
  return filter === 'all' ? clients : clients.filter((client) => client.access === filter)
}

export function displayName(client: AgentClient): string {
  const name = client.name?.trim()
  return name ? name : client.client_id
}

export function kindLabel(client: AgentClient): string {
  return client.kind === 'autonomous' ? 'Autonomous' : 'Delegated'
}

export function canApprove(client: AgentClient): boolean {
  return client.access !== 'approved'
}

export function canBlock(client: AgentClient): boolean {
  return client.access !== 'blocked'
}

export function canUnblock(client: AgentClient): boolean {
  return client.access === 'blocked'
}

export function canDelete(client: AgentClient): boolean {
  return client.source !== 'seen'
}

export function canRotate(client: AgentClient): boolean {
  return client.source === 'autharie'
}

export function lastSeenLabel(iso: string | undefined, now: Date): string {
  if (!iso) return 'Never seen'
  const time = new Date(iso)
  if (Number.isNaN(time.getTime())) return 'Never seen'
  if (now.getTime() - time.getTime() < 60_000) return 'Just now'
  return `${formatDistanceStrict(time, now)} ago`
}

export interface KnownAgent {
  id: string
  label: string
  autonomous: boolean
}

export function knownAgents(clients: AgentClient[]): KnownAgent[] {
  return clients
    .filter((client) => client.access !== 'blocked')
    .map((client) => ({
      id: client.client_id,
      label: displayName(client),
      autonomous: client.kind === 'autonomous',
    }))
}

export function agentLabel(id: string, known: KnownAgent[]): string {
  return known.find((agent) => agent.id === id)?.label ?? id
}

export const REGISTRY_REFRESH_MS = 10_000

export function resolveAgent(text: string, known: KnownAgent[]): string {
  const typed = text.trim()
  if (typed === '') return ''
  const byId = known.find((agent) => agent.id === typed)
  if (byId) return byId.id
  const lowered = typed.toLowerCase()
  const byName = known.filter((agent) => agent.label.toLowerCase() === lowered)
  return byName.length === 1 ? byName[0].id : typed
}
