import { describe, expect, it } from 'vitest'
import { CAN } from '@/domain/organisations/permissions'
import { organisationTabs } from './organisation-tabs'

const path = (suffix = '') => `/organisations/acme${suffix}`

function labels(rights: number[], cloudAccounts: boolean): string[] {
  return organisationTabs({
    can: (bit) => rights.includes(bit),
    organisationPath: path,
    cloudAccounts,
  }).map((tab) => tab.label)
}

describe('the tabs of an organisation', () => {
  it('always has the overview, and nothing else for somebody who may see nothing', () => {
    expect(labels([], true)).toEqual(['Overview'])
  })

  it('shows Cloud accounts to somebody who sees instances, on an installation that connects a provider', () => {
    expect(labels([CAN.viewInstances], true)).toEqual(['Overview', 'Deployments', 'Cloud accounts'])
  })

  it('leaves Cloud accounts out when the installation cannot connect a provider', () => {
    expect(labels([CAN.viewInstances], false)).toEqual(['Overview', 'Deployments'])
  })

  it('leaves Cloud accounts out for somebody who may not see instances, whatever the installation', () => {
    expect(labels([CAN.viewMembers], true)).not.toContain('Cloud accounts')
  })

  it('keeps the order of the tabs', () => {
    const everything = [
      CAN.viewInstances,
      CAN.viewMembers,
      CAN.viewRoles,
      CAN.viewBilling,
    ]

    expect(labels(everything, true)).toEqual([
      'Overview',
      'Deployments',
      'Cloud accounts',
      'Members',
      'Roles',
      'Billing',
    ])
  })

  it('points each tab into the organisation', () => {
    const tabs = organisationTabs({
      can: () => true,
      organisationPath: path,
      cloudAccounts: true,
    })

    expect(tabs.find((tab) => tab.label === 'Cloud accounts')?.to).toBe(path('/cloud-accounts'))
    expect(tabs.find((tab) => tab.label === 'Overview')?.exact).toBe(true)
  })
})
