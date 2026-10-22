import { describe, expect, it } from 'vitest'
import { INITIAL_DRAFT, scenarioOf } from './simulation-draft'

const NOW = new Date('2026-10-22T09:00:00Z')

describe('a simulation draft', () => {
  it('reads a user with groups and roles', () => {
    const scenario = scenarioOf(
      { ...INITIAL_DRAFT, groups: 'support\nops', roles: 'viewer', userId: ' u-7 ', agent: 'claude' },
      NOW,
    )
    expect(scenario.subject).toEqual({
      kind: 'user',
      id: 'u-7',
      groups: ['support', 'ops'],
      roles: ['viewer'],
      agent: 'claude',
    })
  })

  it('reads an autonomous agent', () => {
    const scenario = scenarioOf({ ...INITIAL_DRAFT, kind: 'agent', agent: 'bot' }, NOW)
    expect(scenario.subject).toEqual({ kind: 'agent', clientId: 'bot' })
  })

  it('takes the time as UTC', () => {
    const scenario = scenarioOf({ ...INITIAL_DRAFT, time: '2026-10-19T10:30' }, NOW)
    expect(scenario.time.toISOString()).toBe('2026-10-19T10:30:00.000Z')
  })

  it('uses now when no time is given', () => {
    expect(scenarioOf(INITIAL_DRAFT, NOW).time).toBe(NOW)
  })

  it('uses now when the time is unreadable', () => {
    expect(scenarioOf({ ...INITIAL_DRAFT, time: 'soon' }, NOW).time).toBe(NOW)
  })

  it('falls back to defaults for blank ids', () => {
    const scenario = scenarioOf({ ...INITIAL_DRAFT, userId: ' ', agent: '' }, NOW)
    expect(scenario.subject).toMatchObject({ id: 'user-1', agent: 'my-agent' })
  })
})
