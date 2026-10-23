import { describe, expect, it } from 'vitest'
import {
  conditionLabel,
  destructiveCount,
  subjectCaption,
  subjectLabel,
  targetCaption,
  targetLabel,
} from './canvas-labels'

describe('subjects', () => {
  it('labels each kind', () => {
    expect(subjectLabel({ type: 'any' })).toBe('Any user')
    expect(subjectLabel({ type: 'group', value: 'eng' })).toBe('eng')
    expect(subjectLabel({ type: 'role', value: '' })).toBe('Pick a role')
    expect(subjectCaption({ type: 'agent', value: 'bot' })).toBe('acts on its own')
    expect(subjectCaption({ type: 'any' })).toBe('everyone')
    expect(subjectCaption({ type: 'user', value: 'x' })).toBe('user')
  })
})

describe('targets', () => {
  it('labels read-only and server targets', () => {
    expect(targetLabel({ type: 'read-only', server: null })).toBe('Read-only tools')
    expect(targetCaption({ type: 'read-only', server: null })).toBe('every server')
    expect(targetLabel({ type: 'read-only', server: 'ferriskey' })).toBe('Read-only on FerrisKey')
    expect(targetLabel({ type: 'server', server: 'autharie' })).toBe('Autharie')
    expect(targetCaption({ type: 'server', server: 'autharie' })).toBe('every tool of the server')
  })

  it('counts tools and previews long lists', () => {
    expect(targetLabel({ type: 'tools', tools: [] })).toBe('No tools yet')
    expect(targetLabel({ type: 'tools', tools: ['ferriskey/get_user'] })).toBe('get_user')
    const many = ['a', 'b', 'c', 'd', 'e'].map((name) => `ferriskey/${name}`)
    expect(targetLabel({ type: 'tools', tools: many })).toBe('5 tools')
    expect(targetCaption({ type: 'tools', tools: many })).toBe('a, b, c +2')
    expect(targetCaption({ type: 'tools', tools: many.slice(0, 2) })).toBe('a, b')
  })

  it('counts destructive tools', () => {
    expect(
      destructiveCount({ type: 'tools', tools: ['ferriskey/disable_user', 'ferriskey/get_user'] }),
    ).toBe(1)
    expect(destructiveCount({ type: 'server', server: 'ferriskey' })).toBe(0)
  })
})

describe('conditionLabel', () => {
  it('describes days and hours', () => {
    expect(conditionLabel(null)).toBeNull()
    expect(conditionLabel({ weekdays: [1, 2, 3, 4, 5], hours: { from: 9, to: 18 } })).toBe(
      'Mon–Fri, 09:00–18:00 UTC',
    )
    expect(conditionLabel({ weekdays: [], hours: null })).toBeNull()
  })
})
