import { describe, expect, it } from 'vitest'
import { CAN } from '@/domain/organisations/permissions'
import { identityTabs } from './identity-tabs'

const path = (suffix = '') => `/d${suffix}`

function tabs(rights: number[], ferriskey: boolean, realm = 'acme') {
  return identityTabs({ can: (bit) => rights.includes(bit), deploymentPath: path, ferriskey, realm })
}

describe('the tabs of the Identity space', () => {
  it('lists the realm pages first, then the deployment-wide ones', () => {
    expect(tabs([CAN.manageInstances], true).map((tab) => tab.label)).toEqual([
      'Overview',
      'Applications',
      'Users',
      'Organisations',
      'Roles',
      'Branding',
      'Agents',
      'Webhooks',
    ])
  })

  /**
   * A tab with no page behind it is announced, not linked: following it would
   * land on a screen the router does not have.
   */
  it('announces the pages that do not exist yet instead of linking them', () => {
    const soon = tabs([CAN.manageInstances], true)
      .filter((tab) => tab.soon)
      .map((tab) => tab.label)

    expect(soon).toEqual(['Overview', 'Applications', 'Users', 'Organisations', 'Roles'])
  })

  it('opens Branding, Agents and Webhooks', () => {
    const open = tabs([CAN.manageInstances], true)
      .filter((tab) => !tab.soon)
      .map((tab) => tab.label)

    expect(open).toEqual(['Branding', 'Agents', 'Webhooks'])
  })

  it('keeps the visibility rules the tabs had before they moved', () => {
    expect(tabs([], true).map((tab) => tab.label)).not.toContain('Agents')
    expect(tabs([], true).map((tab) => tab.label)).toContain('Webhooks')
    expect(tabs([CAN.manageInstances], false).map((tab) => tab.label)).not.toContain('Agents')
    expect(tabs([CAN.manageInstances], false).map((tab) => tab.label)).not.toContain('Webhooks')
  })

  it('puts the realm in the address of the pages that belong to one', () => {
    const branding = tabs([], true, 'acme').find((tab) => tab.label === 'Branding')

    expect(branding?.to).toBe('/d/identity/realms/acme/branding')
  })

  it('leaves the deployment-wide pages out of any realm', () => {
    const byLabel = Object.fromEntries(tabs([CAN.manageInstances], true, 'acme').map((tab) => [tab.label, tab.to]))

    expect(byLabel.Agents).toBe('/d/identity/agents')
    expect(byLabel.Webhooks).toBe('/d/identity/webhooks')
  })
})
