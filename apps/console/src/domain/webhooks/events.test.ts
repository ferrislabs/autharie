import { describe, expect, it } from 'vitest'
import { groupEvents, groupSelection, toggleEvent, toggleGroup } from './events'

const CATALOGUE = [
  { name: 'user.created', group: 'user' },
  { name: 'user.deleted', group: 'user' },
  { name: 'client.created', group: 'client' },
]

describe('grouping events', () => {
  it('groups by resource, groups in alphabetical order, events in catalogue order', () => {
    expect(groupEvents(CATALOGUE)).toEqual([
      { group: 'client', events: ['client.created'] },
      { group: 'user', events: ['user.created', 'user.deleted'] },
    ])
  })

  it('filters by event name, ignoring case', () => {
    expect(groupEvents(CATALOGUE, 'DELETED')).toEqual([{ group: 'user', events: ['user.deleted'] }])
  })

  it('keeps a whole group when the search names the group', () => {
    expect(groupEvents(CATALOGUE, 'user')[0].events).toEqual(['user.created', 'user.deleted'])
  })

  it('ignores surrounding spaces', () => {
    expect(groupEvents(CATALOGUE, '  client ')).toHaveLength(1)
  })

  it('answers nothing when nothing matches', () => {
    expect(groupEvents(CATALOGUE, 'zzz')).toEqual([])
  })
})

describe('selecting events', () => {
  const user = { group: 'user', events: ['user.created', 'user.deleted'] }

  it('tells none, some and all apart', () => {
    expect(groupSelection(user, [])).toBe('none')
    expect(groupSelection(user, ['user.created'])).toBe('some')
    expect(groupSelection(user, ['user.created', 'user.deleted', 'x'])).toBe('all')
  })

  it('selects a whole group, keeping what was chosen elsewhere', () => {
    expect(toggleGroup(user, ['client.created', 'user.created'])).toEqual([
      'client.created',
      'user.created',
      'user.deleted',
    ])
  })

  it('clears a whole group only when it was fully selected', () => {
    expect(toggleGroup(user, ['client.created', 'user.created', 'user.deleted'])).toEqual([
      'client.created',
    ])
  })

  it('toggles one event', () => {
    expect(toggleEvent(['a'], 'b')).toEqual(['a', 'b'])
    expect(toggleEvent(['a', 'b'], 'a')).toEqual(['b'])
  })
})
