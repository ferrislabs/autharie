import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { describeEstimate } from '../../estimate'
import { applyMode, initialDraft } from '../../profile'
import type { CloudCredential, ProviderOffers } from '../../types/cloud-provider'
import { CustomerCloudForm, type EstimateView } from './customer-cloud-form'

const offers: ProviderOffers = {
  control_planes: [
    { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
    { id: 'dedicated-4', kind: 'dedicated', monthly_price: 3500 },
  ],
  node_types: [{ node_type: 'DEV1-M', monthly_price: 1500 }],
}

const credential: CloudCredential = {
  id: 'cred-1',
  label: 'prod',
  provider: 'scaleway',
  created_at: '2026-10-01T00:00:00Z',
  scope_checked_at: '2026-10-01T00:00:00Z',
}

const noop = () => {}

function render(mode: 'dev' | 'standard' | 'ha', estimate: EstimateView = { loading: false }) {
  return renderToStaticMarkup(
    <CustomerCloudForm
      credentials={[credential]}
      credentialId='cred-1'
      onCredential={noop}
      regions={['fr-par']}
      region='fr-par'
      onRegion={noop}
      offers={offers}
      offersLoading={false}
      draft={applyMode(initialDraft(offers), mode, offers)}
      onMode={noop}
      onControlPlane={noop}
      onNodeType={noop}
      onMinNodes={noop}
      onMaxNodes={noop}
      estimate={estimate}
    />,
  )
}

function input(html: string, id: string): string {
  return html.match(new RegExp(`<input[^>]*id="${id}"[^>]*>`))![0]
}

describe('the cluster profile form', () => {
  it('fixes the nodes at one in dev', () => {
    const html = render('dev')

    expect(input(html, 'min-nodes')).toContain('disabled=""')
    expect(input(html, 'min-nodes')).toContain('value="1"')
    expect(input(html, 'max-nodes')).toContain('disabled=""')
    expect(html).toContain('mutualized control plane only')
    expect(html).toContain('Replicas: 1')
  })

  it('frees the nodes in standard and ha, with the mode floor pre-filled', () => {
    const standard = render('standard')
    const ha = render('ha')

    expect(input(standard, 'min-nodes')).not.toContain('disabled=""')
    expect(input(standard, 'min-nodes')).toContain('value="2"')
    expect(input(ha, 'min-nodes')).toContain('value="3"')
    expect(ha).toContain('Replicas: 2')
    expect(standard).not.toContain('mutualized control plane only')
  })

  it('shows the estimate with what it does and does not include', () => {
    const html = render('standard', { loading: false, text: describeEstimate({ min: 4500, max: 9000 }) })

    expect(html).toContain(
      'between €45.00 and €90.00 per month, billed by your cloud provider; egress not included',
    )
  })

  it('shows the refusal of the estimate inline', () => {
    const html = render('standard', { loading: false, refusal: 'replication exceeds nodes' })

    expect(html).toContain('replication exceeds nodes')
    expect(html).not.toContain('Estimated cost')
  })
})
