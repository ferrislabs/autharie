import { describe, expect, it } from 'vitest'
import { canStart, canStop, powerItems, STOP_CONFIRMATION } from './power'
import type { DeploymentStatus } from './types/deployment'

const ALL: DeploymentStatus[] = [
  'pending',
  'scheduling',
  'in_progress',
  'successful',
  'failed',
  'deleting',
  'deleted',
  'maintenance',
  'upgrade_required',
  'upgrading',
  'stopped',
]

describe('what can be stopped', () => {
  it('is a deployment that has started and is not being rewritten or removed', () => {
    const stoppable = ALL.filter(canStop)

    expect(stoppable).toEqual(['in_progress', 'successful', 'failed', 'maintenance', 'upgrade_required'])
  })

  it('is nothing while the status is not known', () => {
    expect(canStop(undefined)).toBe(false)
    expect(canStop(null)).toBe(false)
  })
})

describe('what can be started', () => {
  it('is a stopped deployment, and nothing else', () => {
    expect(ALL.filter(canStart)).toEqual(['stopped'])
    expect(canStart(undefined)).toBe(false)
  })

  it('is never what can be stopped', () => {
    for (const status of ALL) {
      expect(canStart(status) && canStop(status), status).toBe(false)
    }
  })
})

describe('the items of the menu', () => {
  it('offers both, and enables the one that applies', () => {
    expect(powerItems('successful', false)).toEqual([
      { action: 'stop', label: 'Stop deployment', disabled: false },
      { action: 'start', label: 'Start deployment', disabled: true },
    ])
    expect(powerItems('stopped', false)).toEqual([
      { action: 'stop', label: 'Stop deployment', disabled: true },
      { action: 'start', label: 'Start deployment', disabled: false },
    ])
  })

  it('disables both while a request is out', () => {
    expect(powerItems('successful', true).every((item) => item.disabled)).toBe(true)
    expect(powerItems('stopped', true).every((item) => item.disabled)).toBe(true)
  })

  it('disables both for a deployment that is being upgraded or deleted', () => {
    expect(powerItems('upgrading', false).every((item) => item.disabled)).toBe(true)
    expect(powerItems('deleting', false).every((item) => item.disabled)).toBe(true)
  })
})

describe('the confirmation to stop', () => {
  it('says the data is kept and the deployment is still billed', () => {
    expect(STOP_CONFIRMATION.description).toContain('data is kept')
    expect(STOP_CONFIRMATION.description).toContain('still billed')
  })
})
