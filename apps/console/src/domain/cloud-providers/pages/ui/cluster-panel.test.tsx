import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { describeEstimate } from '../../estimate'
import { applyMode } from '../../profile'
import { draftFromProfile } from '../../resize'
import type { ProviderOffers } from '../../types/cloud-provider'
import { ClusterPanel } from './cluster-panel'

const offers: ProviderOffers = {
  control_planes: [
    { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
    { id: 'dedicated-4', kind: 'dedicated', monthly_price: 3500 },
  ],
  node_types: [{ node_type: 'PRO2-S', monthly_price: 7200 }],
}

const dev: Schemas.ClusterProfile = {
  mode: 'dev',
  control_plane: { id: 'mutualized', kind: 'mutualized', monthly_price: 0 },
  node_type: 'PRO2-S',
  min_nodes: 1,
  max_nodes: 1,
  replication: 1,
}

const noop = () => {}

function render(overrides: Partial<Parameters<typeof ClusterPanel>[0]> = {}) {
  return renderToStaticMarkup(
    <ClusterPanel
      profile={dev}
      regions={['fr-par']}
      region='fr-par'
      onRegion={noop}
      offers={offers}
      offersLoading={false}
      draft={draftFromProfile(dev)}
      onMode={noop}
      onControlPlane={noop}
      onMinNodes={noop}
      onMaxNodes={noop}
      estimate={{ loading: false }}
      canApply={false}
      applying={false}
      onApply={noop}
      {...overrides}
    />,
  )
}

function tag(html: string, pattern: string): string {
  return html.match(new RegExp(pattern))![0]
}

describe('the cluster panel', () => {
  it('shows the profile in force and the node type read-only', () => {
    const html = render()

    expect(html).toContain('Now: Dev, 1 node of PRO2-S, 1 replica')
    expect(tag(html, '<input[^>]*id="cluster-node-type"[^>]*>')).toContain('readOnly=""')
    expect(tag(html, '<input[^>]*id="cluster-node-type"[^>]*>')).toContain('value="PRO2-S"')
    expect(html).toContain('cannot change')
  })

  it('locks the nodes in dev and frees them in standard with the floor pre-filled', () => {
    const lockedHtml = render()
    const standard = render({ draft: applyMode(draftFromProfile(dev), 'standard', offers) })

    expect(tag(lockedHtml, '<input[^>]*id="cluster-min-nodes"[^>]*>')).toContain('disabled=""')
    expect(tag(standard, '<input[^>]*id="cluster-min-nodes"[^>]*>')).not.toContain('disabled=""')
    expect(tag(standard, '<input[^>]*id="cluster-min-nodes"[^>]*>')).toContain('value="2"')
    expect(standard).toContain('Replicas: 2')
  })

  it('disables Apply until the draft can be applied', () => {
    expect(tag(render(), '<button[^>]*>Apply</button>')).toContain('disabled=""')
    expect(tag(render({ canApply: true }), '<button[^>]*>Apply</button>')).not.toContain('disabled=""')
    expect(render({ canApply: true, applying: true })).toContain('Applying…')
  })

  it('shows the estimate and its refusal inline', () => {
    expect(render({ estimate: { loading: false, text: describeEstimate({ min: 4500, max: 9000 }) } })).toContain(
      'between €45.00 and €90.00 per month',
    )
    expect(render({ estimate: { loading: false, refusal: 'node type not offered' } })).toContain(
      'node type not offered',
    )
  })

  it('warns before taking running replicas away', () => {
    const ha: Schemas.ClusterProfile = { ...dev, mode: 'ha', min_nodes: 3, max_nodes: 5, replication: 2 }
    const html = render({ profile: ha, draft: applyMode(draftFromProfile(ha), 'dev', offers) })

    expect(html).toContain('2 replicas are running and Dev would allow 1.')
  })

  it('shows the server refusal inline, and the new profile on success', () => {
    const refused = render({ failure: '2 replicas are running and dev allows 1' })
    const resized = render({ applied: { ...dev, mode: 'standard', min_nodes: 2, max_nodes: 4, replication: 2 } })

    expect(refused).toContain('2 replicas are running and dev allows 1')
    expect(refused).not.toContain('Resized.')
    expect(resized).toContain('Resized. The cluster now runs Standard, 2 to 4 nodes of PRO2-S, 2 replicas')
  })
})
