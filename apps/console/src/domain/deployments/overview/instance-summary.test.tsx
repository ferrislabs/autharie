import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { InstanceSummary, type InstanceFacts } from './instance-summary'

function facts(over: Partial<InstanceFacts> = {}): InstanceFacts {
  return {
    kind: 'ferriskey',
    version: '1.4.0',
    offer: 'Standard',
    resources: '1 vCPU · 2 GiB · 5 GiB database',
    runsOn: 'Shared cluster',
    createdAt: Date.parse('2026-10-01T00:00:00Z'),
    ...over,
  }
}

function render(over: Partial<InstanceFacts> = {}) {
  return renderToStaticMarkup(<InstanceSummary facts={facts(over)} />)
}

describe('the provider in the instance section', () => {
  it('shows the FerrisKey logo, then its name', () => {
    const html = render({ kind: 'ferriskey' })
    expect(html).toContain('Provider')
    expect(html).toContain('FerrisKey')
    expect(html.indexOf('<img')).toBeGreaterThanOrEqual(0)
    expect(html.indexOf('<img')).toBeLessThan(html.indexOf('FerrisKey'))
  })

  it('shows the Keycloak logo, then its name', () => {
    const html = render({ kind: 'keycloak' })
    expect(html).toContain('Keycloak')
    expect(html.indexOf('<svg')).toBeGreaterThanOrEqual(0)
    expect(html.indexOf('<svg')).toBeLessThan(html.indexOf('Keycloak'))
  })

  it('comes first, before what the instance is reached at', () => {
    const html = render({ endpoint: 'https://acme.example.test' })
    expect(html.indexOf('Provider')).toBeLessThan(html.indexOf('Endpoint'))
  })
})
