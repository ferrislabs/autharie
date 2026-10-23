import { describe, expect, it } from 'vitest'
import {
  conditionLabel,
  destructiveCount,
  isDestructiveTarget,
  serverLabel,
  subjectCaption,
  subjectLabel,
  targetCaption,
  targetLabel,
  unknownServers,
} from './canvas-labels'
import { BUILTIN_CATALOGUE, catalogueOf } from './tool-catalogue'

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
    expect(targetLabel({ type: 'read-only', server: null }, BUILTIN_CATALOGUE)).toBe('Read-only tools')
    expect(targetCaption({ type: 'read-only', server: null })).toBe('every server')
    expect(targetLabel({ type: 'read-only', server: 'ferriskey' }, BUILTIN_CATALOGUE)).toBe('Read-only on FerrisKey')
    expect(targetLabel({ type: 'server', server: 'autharie' }, BUILTIN_CATALOGUE)).toBe('Autharie')
    expect(targetCaption({ type: 'server', server: 'autharie' })).toBe('every tool of the server')
  })

  it('counts tools and previews long lists', () => {
    expect(targetLabel({ type: 'tools', tools: [] }, BUILTIN_CATALOGUE)).toBe('No tools yet')
    expect(targetLabel({ type: 'tools', tools: ['ferriskey/get_user'] }, BUILTIN_CATALOGUE)).toBe('get_user')
    const many = ['a', 'b', 'c', 'd', 'e'].map((name) => `ferriskey/${name}`)
    expect(targetLabel({ type: 'tools', tools: many }, BUILTIN_CATALOGUE)).toBe('5 tools')
    expect(targetCaption({ type: 'tools', tools: many })).toBe('a, b, c +2')
    expect(targetCaption({ type: 'tools', tools: many.slice(0, 2) })).toBe('a, b')
  })

  it('counts destructive tools', () => {
    expect(
      destructiveCount({ type: 'tools', tools: ['ferriskey/disable_user', 'ferriskey/get_user'] }, BUILTIN_CATALOGUE),
    ).toBe(1)
    expect(destructiveCount({ type: 'server', server: 'ferriskey' }, BUILTIN_CATALOGUE)).toBe(0)
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

describe('customer servers', () => {
  const crm = catalogueOf([
    {
      slug: 'crm',
      name: 'CRM',
      discovery: {
        tools: [
          { name: 'list_invoices', read_only: true, destructive: false },
          { name: 'delete_invoice', read_only: false, destructive: true },
        ],
      },
    },
  ])

  it('labels a customer server by its name', () => {
    expect(serverLabel(crm, 'crm')).toBe('CRM')
    expect(targetLabel({ type: 'server', server: 'crm' }, crm)).toBe('CRM')
    expect(targetLabel({ type: 'read-only', server: 'crm' }, crm)).toBe('Read-only on CRM')
  })

  it('knows which customer tools are destructive', () => {
    const tools = ['crm/list_invoices', 'crm/delete_invoice']
    expect(destructiveCount({ type: 'tools', tools }, crm)).toBe(1)
    expect(isDestructiveTarget({ type: 'tools', tools: ['crm/delete_invoice'] }, crm)).toBe(true)
    expect(destructiveCount({ type: 'tools', tools }, BUILTIN_CATALOGUE)).toBe(0)
  })

  it('marks a server that is no longer registered, once the list is known', () => {
    const target = { type: 'server', server: 'erp' } as const

    expect(serverLabel(crm, 'erp')).toBe('erp (unknown server)')
    expect(targetLabel(target, crm)).toBe('erp (unknown server)')
    expect(unknownServers(target, crm)).toEqual(['erp'])
    expect(unknownServers({ type: 'tools', tools: ['erp/x', 'crm/y', 'erp/z'] }, crm)).toEqual(['erp'])
    expect(unknownServers({ type: 'read-only', server: null }, crm)).toEqual([])
  })

  it('does not call a server unknown while the list is still loading', () => {
    expect(serverLabel(BUILTIN_CATALOGUE, 'crm')).toBe('crm')
    expect(unknownServers({ type: 'server', server: 'crm' }, BUILTIN_CATALOGUE)).toEqual([])
    expect(unknownServers({ type: 'server', server: 'crm' }, catalogueOf([], false))).toEqual([])
  })
})
