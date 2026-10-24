import { describe, expect, it } from 'vitest'
import { CAN } from '@/domain/organisations/permissions'
import { deploymentTabs } from './deployment-tabs'

const path = (suffix = '') => `/d${suffix}`

function labels(rights: number[]): string[] {
  return deploymentTabs({ can: (bit) => rights.includes(bit), deploymentPath: path }).map(
    (tab) => tab.label,
  )
}

describe('the tabs of a deployment', () => {
  it('holds infrastructure only: what moved to Identity is not here any more', () => {
    const everything = labels([CAN.viewBackups, CAN.manageInstances])

    expect(everything).toEqual(['Overview', 'Observability', 'Backups', 'Security', 'Settings'])
    for (const moved of ['Branding', 'Webhooks', 'Agents']) {
      expect(everything).not.toContain(moved)
    }
  })

  it('leaves Backups out for somebody who may not see them', () => {
    expect(labels([])).toEqual(['Overview', 'Observability', 'Security', 'Settings'])
  })
})
