import { describe, expect, it } from 'vitest'
import {
  agentLabel,
  blockedCount,
  canApprove,
  canBlock,
  canDelete,
  canRotate,
  canUnblock,
  displayName,
  filterClients,
  kindLabel,
  knownAgents,
  lastSeenLabel,
  pendingCount,
  resolveAgent,
} from './registry'
import type { AgentClient } from './types'

function client(over: Partial<AgentClient> = {}): AgentClient {
  return {
    client_id: 'dcr-1',
    source: 'dynamic',
    kind: 'delegated',
    access: 'approved',
    ...over,
  }
}

const NOW = new Date('2026-10-23T12:00:00Z')

describe('the registry helpers', () => {
  it('counts and filters pending and blocked agents', () => {
    const clients = [
      client({ client_id: 'a', access: 'pending' }),
      client({ client_id: 'b', access: 'pending' }),
      client({ client_id: 'c', access: 'blocked' }),
      client({ client_id: 'd' }),
    ]

    expect(pendingCount(clients)).toBe(2)
    expect(blockedCount(clients)).toBe(1)
    expect(filterClients(clients, 'pending').map((c) => c.client_id)).toEqual(['a', 'b'])
    expect(filterClients(clients, 'blocked').map((c) => c.client_id)).toEqual(['c'])
    expect(filterClients(clients, 'all')).toHaveLength(4)
  })

  it('shows the name, or the client id when there is none', () => {
    expect(displayName(client({ name: ' Claude ' }))).toBe('Claude')
    expect(displayName(client({ name: ' ' }))).toBe('dcr-1')
    expect(displayName(client())).toBe('dcr-1')
  })

  it('names the kind', () => {
    expect(kindLabel(client())).toBe('Delegated')
    expect(kindLabel(client({ kind: 'autonomous' }))).toBe('Autonomous')
  })

  it('offers only the actions that make sense for the access state', () => {
    const pending = client({ access: 'pending' })
    const approved = client({ access: 'approved' })
    const blocked = client({ access: 'blocked' })

    expect([canApprove(pending), canBlock(pending), canUnblock(pending)]).toEqual([true, true, false])
    expect([canApprove(approved), canBlock(approved), canUnblock(approved)]).toEqual([false, true, false])
    expect([canApprove(blocked), canBlock(blocked), canUnblock(blocked)]).toEqual([true, false, true])
  })

  it('lets only created agents rotate and never deletes what was only seen', () => {
    expect(canRotate(client({ source: 'autharie' }))).toBe(true)
    expect(canRotate(client())).toBe(false)
    expect(canDelete(client({ source: 'seen' }))).toBe(false)
    expect(canDelete(client())).toBe(true)
  })

  it('says when an agent was last seen', () => {
    expect(lastSeenLabel(undefined, NOW)).toBe('Never seen')
    expect(lastSeenLabel('not a date', NOW)).toBe('Never seen')
    expect(lastSeenLabel('2026-10-23T11:59:30Z', NOW)).toBe('Just now')
    expect(lastSeenLabel('2026-10-23T11:55:00Z', NOW)).toBe('5 minutes ago')
    expect(lastSeenLabel('2026-10-21T12:00:00Z', NOW)).toBe('2 days ago')
  })

  it('offers every agent that is not blocked to the canvas, by name', () => {
    const known = knownAgents([
      client({ client_id: 'a', name: 'Claude' }),
      client({ client_id: 'b', access: 'blocked' }),
      client({ client_id: 'agent-n8n-abc', kind: 'autonomous', source: 'autharie' }),
    ])

    expect(known).toEqual([
      { id: 'a', label: 'Claude', autonomous: false },
      { id: 'agent-n8n-abc', label: 'agent-n8n-abc', autonomous: true },
    ])
    expect(agentLabel('a', known)).toBe('Claude')
    expect(agentLabel('free-text', known)).toBe('free-text')
  })

  it('resolves a typed name to its id and keeps free text as typed', () => {
    const known = [
      { id: 'c-1', label: 'Claude', autonomous: false },
      { id: 'c-2', label: 'Twin', autonomous: false },
      { id: 'c-3', label: 'twin', autonomous: false },
    ]

    expect(resolveAgent('c-1', known)).toBe('c-1')
    expect(resolveAgent(' claude ', known)).toBe('c-1')
    expect(resolveAgent('Twin', known)).toBe('Twin')
    expect(resolveAgent('something-else', known)).toBe('something-else')
    expect(resolveAgent('  ', known)).toBe('')
  })
})
