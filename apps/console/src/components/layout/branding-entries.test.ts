import { describe, expect, it } from 'vitest'
import { brandingEntries } from './branding-entries'

const entries = brandingEntries((path) => `/d${path}`)

describe('the Branding side navigation', () => {
  it('lists Auth0 entries in Auth0 order', () => {
    expect(entries.map((entry) => entry.label)).toEqual([
      'Universal login',
      'Custom domains',
      'Email templates',
      'Email provider',
      'Phone templates',
      'Phone provider',
    ])
  })

  it('opens Universal login and Email provider and announces the rest', () => {
    const open = entries.filter((entry) => !('soon' in entry && entry.soon))

    expect(open.map((entry) => entry.label)).toEqual([
      'Universal login',
      'Email provider',
    ])
  })

  it('builds each address from the deployment path', () => {
    expect(entries[0]).toMatchObject({ to: '/d/branding/universal-login' })
    expect(entries[3]).toMatchObject({ to: '/d/branding/email-provider' })
  })
})
