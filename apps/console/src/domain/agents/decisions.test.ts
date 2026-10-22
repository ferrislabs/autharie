import { describe, expect, it } from 'vitest'
import {
  DECISIONS_LIMIT,
  decisionKey,
  policyLabel,
  buildDecisionQuery,
  decisionBadge,
  emptyFilters,
  hasFilters,
  isAllow,
  subjectLabel,
} from './decisions'
import type { AgentDecision, AgentPolicy } from './types'

const NOW = new Date('2026-10-22T12:00:00.000Z')

describe('the decision query', () => {
  it('asks for the last 24 hours and nothing else by default', () => {
    expect(buildDecisionQuery(emptyFilters(), NOW)).toEqual({
      from: '2026-10-21T12:00:00.000Z',
      to: '2026-10-22T12:00:00.000Z',
      limit: DECISIONS_LIMIT,
    })
  })

  it('filters on the decision', () => {
    expect(buildDecisionQuery({ ...emptyFilters(), decision: 'deny' }, NOW).decision).toBe('deny')
  })

  it('leaves the decision out for all', () => {
    expect(buildDecisionQuery({ ...emptyFilters(), decision: 'all' }, NOW)).not.toHaveProperty(
      'decision'
    )
  })

  it('trims tool and subject', () => {
    const query = buildDecisionQuery(
      { ...emptyFilters(), tool: ' autharie/list_backups ', subject: ' u-1 ' },
      NOW
    )
    expect(query.tool).toBe('autharie/list_backups')
    expect(query.subject).toBe('u-1')
  })

  it('leaves blank text out', () => {
    const query = buildDecisionQuery({ ...emptyFilters(), tool: '  ', subject: '' }, NOW)
    expect(query).not.toHaveProperty('tool')
    expect(query).not.toHaveProperty('subject')
  })

  it.each([
    ['1h', '2026-10-22T11:00:00.000Z'],
    ['24h', '2026-10-21T12:00:00.000Z'],
    ['7d', '2026-10-15T12:00:00.000Z'],
  ] as const)('looks back %s', (range, from) => {
    expect(buildDecisionQuery({ ...emptyFilters(), range }, NOW).from).toBe(from)
  })
})

describe('knowing a filter is set', () => {
  it('is false for the defaults', () => {
    expect(hasFilters(emptyFilters())).toBe(false)
  })

  it('is true for each filter', () => {
    expect(hasFilters({ ...emptyFilters(), decision: 'allow' })).toBe(true)
    expect(hasFilters({ ...emptyFilters(), tool: 'x' })).toBe(true)
    expect(hasFilters({ ...emptyFilters(), subject: 'x' })).toBe(true)
  })

  it('ignores the time range, which always applies', () => {
    expect(hasFilters({ ...emptyFilters(), range: '7d' })).toBe(false)
  })
})

describe('reading a decision', () => {
  it.each([
    ['allow', true],
    ['Allow', true],
    [true, true],
    ['deny', false],
    [false, false],
    ['error', false],
  ] as const)('reads %s as allow=%s', (value, allowed) => {
    expect(isAllow(value)).toBe(allowed)
  })

  it('badges allow as success and deny as danger', () => {
    expect(decisionBadge('allow')).toEqual({ tone: 'success', label: 'Allow' })
    expect(decisionBadge('deny')).toEqual({ tone: 'danger', label: 'Deny' })
  })
})

describe('naming the subject', () => {
  const base = { subject_type: 'User', subject_id: 'u-1' } as AgentDecision

  it('prefixes the type', () => {
    expect(subjectLabel(base)).toBe('User u-1')
  })

  it('says Client for a client', () => {
    expect(subjectLabel({ ...base, subject_type: 'Client', subject_id: 'bot' })).toBe('Client bot')
  })
})

describe('naming the policies behind a decision', () => {
  const policies = [{ id: 'p-1', name: 'Read-only tools' }] as AgentPolicy[]

  it('shows the name of a policy written through Autharie', () => {
    expect(policyLabel('autharie:p-1', policies)).toBe('Read-only tools')
  })

  it('keeps a name it does not know', () => {
    expect(policyLabel('autharie:p-2', policies)).toBe('autharie:p-2')
    expect(policyLabel('members-use-tools', policies)).toBe('members-use-tools')
  })
})

describe('keying decisions', () => {
  it('tells apart the tools of one list request', () => {
    const base = { request_id: 'r-1', method: 'tools/list' } as AgentDecision
    expect(decisionKey({ ...base, tool: 'a' }, 0)).not.toBe(decisionKey({ ...base, tool: 'b' }, 1))
  })
})
