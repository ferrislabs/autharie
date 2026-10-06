import { describe, expect, it } from 'vitest'
import {
  describePrice,
  describeVolume,
  formatAmount,
  summaryLines,
} from './summary'
import { ferriskeyOnlyFeatures, KEYCLOAK_NOTICE, volumeNotes } from './plans'

describe('the price line', () => {
  it('says Free for nothing', () => {
    expect(describePrice({ kind: 'price', amount: 0 })).toBe('Free')
    expect(formatAmount({ kind: 'price', amount: 0 })).toBe('Free')
  })

  it('gives euros excluding VAT for a paid price', () => {
    expect(describePrice({ kind: 'price', amount: 2690 })).toBe('€2,690 per month, excl. VAT')
  })
})

describe('the summary', () => {
  it('has one line for a managed deployment', () => {
    expect(summaryLines('managed', { kind: 'price', amount: 99 })).toEqual([
      { label: 'Autharie', value: '€99 per month, excl. VAT' },
    ])
  })

  it('keeps the cluster on its own line for a deployment in your cloud', () => {
    expect(summaryLines('byoc', { kind: 'price', amount: 269 })).toEqual([
      { label: 'Autharie', value: '€269 per month, excl. VAT' },
      { label: 'Your cluster', value: 'Estimated above, billed by your cloud provider' },
    ])
  })

  it('carries the cluster estimate when there is one', () => {
    const estimate = 'between €30.00 and €40.00 per month, billed by your cloud provider; egress not included'
    expect(summaryLines('byoc', { kind: 'price', amount: 70 }, estimate)[1]).toEqual({
      label: 'Your cluster',
      value: estimate,
    })
  })

  it('says unlimited for a deployment in your cloud', () => {
    expect(describeVolume('byoc', 1000)).toBe('Unlimited accounts')
    expect(describeVolume('managed', 25000)).toBe('25,000 accounts')
  })
})

describe('the Keycloak notice', () => {
  it('says what is missing and offers FerrisKey', () => {
    expect(KEYCLOAK_NOTICE.warning).toBe(
      'Distributed ReBAC and the MCP Hub are not available on this engine.',
    )
    expect(KEYCLOAK_NOTICE.switch).toBe('Switch to FerrisKey')
    expect(ferriskeyOnlyFeatures('managed')).toEqual([
      'Distributed authorisations (ReBAC) included',
      'MCP gateway (AI and agents) included',
    ])
    expect(ferriskeyOnlyFeatures('byoc')).toEqual(['ReBAC and MCP included'])
  })
})

describe('the volume notes', () => {
  it('says the free offer is FerrisKey only', () => {
    expect(volumeNotes('managed', 'keycloak', 1000)[0]).toMatch(/reserved for the FerrisKey/)
    expect(volumeNotes('managed', 'ferriskey', 1000)[0]).toMatch(/free up to 1,000/)
    expect(volumeNotes('byoc', 'ferriskey', 1000)).toEqual([])
    expect(volumeNotes('managed', 'ferriskey', 25000)).toEqual([])
  })
})
