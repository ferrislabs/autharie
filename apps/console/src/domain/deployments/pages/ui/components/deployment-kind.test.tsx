import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { DeploymentKind } from '../../../types/deployment'
import { DeploymentKindLabel } from './deployment-kind'

const LOGO_TAG: Record<DeploymentKind, string> = {
  ferriskey: '<img',
  keycloak: '<svg',
}

const NAME: Record<DeploymentKind, string> = {
  ferriskey: 'FerrisKey',
  keycloak: 'Keycloak',
}

describe('DeploymentKindLabel', () => {
  it.each(['ferriskey', 'keycloak'] as const)('shows the %s logo, then its name', (kind) => {
    const html = renderToStaticMarkup(<DeploymentKindLabel kind={kind} />)

    expect(html).toContain(NAME[kind])
    expect(html.indexOf(LOGO_TAG[kind])).toBeGreaterThanOrEqual(0)
    expect(html.indexOf(LOGO_TAG[kind])).toBeLessThan(html.indexOf(NAME[kind]))
  })

  it('does not announce the logo on top of the name', () => {
    const html = renderToStaticMarkup(<DeploymentKindLabel kind='ferriskey' />)

    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('alt=""')
  })

  it.each(['ferriskey', 'keycloak'] as const)('draws the %s logo at 24 px', (kind) => {
    expect(renderToStaticMarkup(<DeploymentKindLabel kind={kind} />)).toContain('size-6')
  })
})
