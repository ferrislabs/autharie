import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { OrganisationQuotaView } from '../../types'
import { OrganisationQuotas } from './organisation-quotas'

const deployments: OrganisationQuotaView = {
  kind: 'deployments',
  description: 'Deployments of the organisation.',
  plan_default: { type: 'limited', max: 3 },
  override: null,
  effective: { type: 'limited', max: 3 },
  held: 2,
}

const render = (props: Partial<Parameters<typeof OrganisationQuotas>[0]> = {}) =>
  renderToStaticMarkup(
    <OrganisationQuotas
      quotas={[deployments]}
      isLoading={false}
      canChange
      onChange={vi.fn()}
      {...props}
    />
  )

describe('the organisation quotas section', () => {
  it('shows the kind, its description, its default and what is held', () => {
    const html = render()

    expect(html).toContain('Organisation quotas')
    expect(html).toContain('deployments')
    expect(html).toContain('Deployments of the organisation.')
    expect(html).toContain('Plan default: 3')
    expect(html).toContain('2 of 3')
  })

  it('shows an override', () => {
    const html = render({
      quotas: [
        { ...deployments, override: { type: 'unlimited' }, effective: { type: 'unlimited' } },
      ],
    })

    expect(html).toContain('Override')
    expect(html).toContain('>Unlimited<')
  })

  it('disables every control without the right to change', () => {
    const html = render({ canChange: false })

    expect(html.match(/<button[^>]*disabled=""/g)?.length).toBe(3)
    expect(html).toMatch(/<input[^>]*disabled=""/)
  })

  it('offers a reset only where an override exists', () => {
    const without = render()
    const withOverride = render({
      quotas: [{ ...deployments, override: { type: 'limited', max: 9 } }],
    })

    expect(without).toMatch(/<button[^>]*disabled=""[^>]*>Reset to default/)
    expect(withOverride).not.toMatch(/<button[^>]*disabled=""[^>]*>Reset to default/)
  })

  it('shows a refusal', () => {
    expect(render({ refusal: 'No.' })).toContain('No.')
  })
})
