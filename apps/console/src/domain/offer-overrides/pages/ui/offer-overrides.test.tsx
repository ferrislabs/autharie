import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { OfferOverrideView } from '../../types'
import { OfferOverrides } from './offer-overrides'

const scale: OfferOverrideView = {
  offer: 'scale',
  plan_default: false,
  override: true,
  effective: true,
}

const render = (props: Partial<Parameters<typeof OfferOverrides>[0]> = {}) =>
  renderToStaticMarkup(
    <OfferOverrides offers={[scale]} isLoading={false} canChange onChoose={vi.fn()} {...props} />
  )

describe('the offers section', () => {
  it('shows the plan default and the effective state', () => {
    const html = render()

    expect(html).toContain('scale')
    expect(html).toContain('Plan default: Closed')
    expect(html).toContain('>Open</span>')
  })

  it('selects the override in the list', () => {
    expect(render()).toMatch(/<option value="open" selected=""/)
    expect(render({ offers: [{ ...scale, override: null }] })).toMatch(
      /<option value="default" selected=""/
    )
  })

  it('carries the billing note', () => {
    expect(render()).toContain('billed at 0 € until paid tiers exist')
  })

  it('disables the select without the right to change', () => {
    expect(render({ canChange: false })).toMatch(/<select[^>]*disabled=""/)
  })
})
