import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { FeatureOverview, TenantFeature } from '../../types/feature'
import { PageFeatures } from './page-features'
import { TenantFeatures } from './tenant-features'

const noop = () => {}

const overview: FeatureOverview = {
  key: 'customer_cloud',
  description: 'Deploy into the own cloud account.',
  enabled_by_default: true,
  overrides_on: 2,
  overrides_off: 1,
}

const tenant = (override: boolean | null): TenantFeature => ({
  key: 'customer_cloud',
  description: 'Deploy into the own cloud account.',
  default: false,
  override,
  effective: override === true,
})

describe('the features page', () => {
  const render = (props: Partial<Parameters<typeof PageFeatures>[0]> = {}) =>
    renderToStaticMarkup(
      <PageFeatures features={[overview]} isLoading={false} canChange onToggle={noop} {...props} />
    )

  it('lists each flag with its description and override counts', () => {
    const html = render()

    expect(html).toContain('customer_cloud')
    expect(html).toContain('Deploy into the own cloud account.')
    expect(html).toContain('>2</td>')
    expect(html).toContain('>1</td>')
  })

  it('shows the default as a switch that is on', () => {
    const html = render()

    expect(html).toContain('aria-label="Default for customer_cloud"')
    expect(html).toContain('aria-checked="true"')
  })

  it('disables the switch for an operator without the right', () => {
    const html = render({ canChange: false })

    expect(html).toMatch(/<button[^>]*role="switch"[^>]*disabled/)
  })

  it('says so when there are no flags', () => {
    expect(render({ features: [] })).toContain('No features')
  })

  it('repeats a refusal', () => {
    expect(render({ refusal: 'forbidden: operate_fleet' })).toContain('forbidden: operate_fleet')
  })
})

describe('the organisation features section', () => {
  const render = (features: TenantFeature[], canChange = true) =>
    renderToStaticMarkup(
      <TenantFeatures features={features} isLoading={false} canChange={canChange} onChoose={noop} />
    )

  it('offers default, on and off for each flag', () => {
    const html = render([tenant(null)])

    expect(html).toContain('>Default</button>')
    expect(html).toContain('>On</button>')
    expect(html).toContain('>Off</button>')
  })

  it('marks the current choice as pressed', () => {
    const pressed = (html: string) =>
      [...html.matchAll(/aria-pressed="true"[^>]*>([^<]*)</g)].map((match) => match[1])

    expect(pressed(render([tenant(null)]))).toEqual(['Default'])
    expect(pressed(render([tenant(true)]))).toEqual(['On'])
    expect(pressed(render([tenant(false)]))).toEqual(['Off'])
  })

  it('shows the default and the effective value', () => {
    const html = render([tenant(true)])

    expect(html).toContain('Default off')
    expect(html).toContain('>on</span>')
  })

  it('disables the controls for an operator without the right', () => {
    expect(render([tenant(null)], false)).toMatch(/disabled=""[^>]*>Default<\/button>/)
  })
})
