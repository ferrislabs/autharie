import { describe, expect, it } from 'vitest'
import { isRedirect } from '@tanstack/react-router'
import { router } from './router'

const ORGANISATION = '/organisations/org-1'
const DEPLOYMENT = `${ORGANISATION}/deployments/dep-1`

/**
 * The components a URL puts on screen, outermost first.
 *
 * Read from the tree rather than from a rendered page: what matters here is
 * which shells a path resolves to, and asking the router that directly needs
 * no browser to be standing up.
 */
function shellsFor(pathname: string): string[] {
  return router
    .getMatchedRoutes(pathname)
    .matchedRoutes.map((route) => route.options?.component)
    .filter((component) => typeof component === 'function')
    .map((component) => (component as { name?: string }).name ?? '')
}

describe('the route tree', () => {
  it('puts the cloud accounts page inside the organisation shell', () => {
    expect(shellsFor(`${ORGANISATION}/cloud-accounts`)).toContain('AppLayout')
    expect(shellsFor(`${ORGANISATION}/cloud-accounts`)).toContain('PageCloudCredentialsFeature')
  })

  it('gives an organisation page the organisation shell', () => {
    expect(shellsFor(`${ORGANISATION}/deployments`)).toContain('AppLayout')
  })

  /**
   * The bug this exists to keep out: nested, the deployment's header rendered
   * inside the organisation's, and the page opened with two stacked headers
   * and two rows of tabs.
   */
  it('replaces the organisation shell rather than nesting inside it', () => {
    const shells = shellsFor(DEPLOYMENT)

    expect(shells).toContain('DeploymentLayout')
    expect(shells).not.toContain('AppLayout')
  })

  it('keeps the deployment shell on every page beneath it', () => {
    for (const path of [
      '/observability',
      '/observability/traces',
      '/observability/usage',
      '/settings',
      '/settings/version',
      '/backups',
      '/security',
    ]) {
      const shells = shellsFor(`${DEPLOYMENT}${path}`)

      expect(shells, path).toContain('DeploymentLayout')
      expect(shells, path).not.toContain('AppLayout')
    }
  })

  /**
   * Members belong to the organisation, not to a deployment. Nested under one
   * they would read as that deployment's members, which is not a thing.
   */
  it('keeps members in the organisation shell rather than a deployment one', () => {
    const shells = shellsFor(`${ORGANISATION}/members`)

    // The leaf, not only the shell: without it the assertion passes for a
    // path that resolves to the organisation's index because the members
    // route was moved somewhere the URL no longer reaches.
    expect(shells).toContain('PageMembersFeature')
    expect(shells).toContain('AppLayout')
    expect(shells).not.toContain('DeploymentLayout')
  })

  /**
   * Under no organisation shell at all. Nested in one it would ask somebody
   * to already be where the link is meant to take them.
   */
  it('puts the invitation link outside every organisation', () => {
    const shells = shellsFor('/invitations/accept')

    expect(shells).toContain('PageAcceptInvitationFeature')
    expect(shells).not.toContain('AppLayout')
    expect(shells).not.toContain('DeploymentLayout')
  })

  it('keeps roles in the organisation shell, beside members', () => {
    const shells = shellsFor(`${ORGANISATION}/roles`)

    expect(shells).toContain('PageRolesFeature')
    expect(shells).toContain('AppLayout')
    expect(shells).not.toContain('DeploymentLayout')
  })

  it('puts the settings navigation only on settings pages', () => {
    expect(shellsFor(`${DEPLOYMENT}/settings`)).toContain('DeploymentSettingsLayout')
    expect(shellsFor(`${DEPLOYMENT}/settings/resources`)).toContain('DeploymentSettingsLayout')
    expect(shellsFor(`${DEPLOYMENT}/observability`)).not.toContain('DeploymentSettingsLayout')
    for (const path of ['/backups', '/security']) {
      expect(shellsFor(`${DEPLOYMENT}${path}`), path).not.toContain('DeploymentSettingsLayout')
    }
  })

  /**
   * Logs, traces and usage share one side nav now instead of three separate
   * top tabs -- asserted the same way settings' own nav is, on the leaf
   * rather than only on the shell (see that test's own comment on why).
   */
  it('puts the observability navigation on all three of its pages, and nowhere else', () => {
    expect(shellsFor(`${DEPLOYMENT}/observability`)).toContain('DeploymentObservabilityLayout')
    expect(shellsFor(`${DEPLOYMENT}/observability/traces`)).toContain(
      'DeploymentObservabilityLayout',
    )
    expect(shellsFor(`${DEPLOYMENT}/observability/usage`)).toContain(
      'DeploymentObservabilityLayout',
    )
    expect(shellsFor(`${DEPLOYMENT}/settings`)).not.toContain('DeploymentObservabilityLayout')
  })

  /// Asserted on the leaf, not on the shell. A route re-nested under the wrong
  /// parent still puts the settings navigation on screen, and a test that only
  /// checked for that would have passed.
  it('opens the backups screen as a tab of its own, not under the settings navigation', () => {
    const shells = shellsFor(`${DEPLOYMENT}/backups`)

    expect(shells).toContain('PageBackupsFeature')
    expect(shells).not.toContain('DeploymentSettingsLayout')
  })

  /**
   * `create` and a deployment id sit at the same depth, so the tree has to
   * prefer the literal. Ranked the other way, creating a deployment would
   * open a deployment called "create".
   */
  it('reads /deployments/create as the form, not as a deployment', () => {
    const shells = shellsFor(`${ORGANISATION}/deployments/create`)

    expect(shells).toContain('AppLayout')
    expect(shells).not.toContain('DeploymentLayout')
  })

  /**
   * A section, not a screen. Without this it rendered the header and an empty
   * body, which looks like a page that failed to load.
   */
  it('sends bare /platform somewhere rather than nowhere', () => {
    const match = router.getMatchedRoutes('/platform')

    expect(match.foundRoute?.options.beforeLoad).toBeDefined()
  })

  it('gives the platform pages their own shell, outside any organisation', () => {
    const shells = shellsFor('/platform/dataplanes')

    expect(shells).toContain('PlatformLayout')
    expect(shells).not.toContain('AppLayout')
  })

  /**
   * Every tab in the platform section resolves. A tab pointing at a route
   * nobody registered renders the shell and an empty body, which reads as a
   * page that failed rather than as a link that was never wired.
   */
  it('resolves every platform screen', () => {
    for (const path of [
      '/platform/deployments',
      '/platform/organisations',
      '/platform/organisations/00000000-0000-0000-0000-000000000000',
      '/platform/dataplanes',
      '/platform/dataplane-upgrades',
      '/platform/releases',
      '/platform/features',
      '/platform/trail',
    ]) {
      expect(router.getMatchedRoutes(path).foundRoute, path).toBeDefined()
    }
  })

  /**
   * Narrowing the estate to one organisation is a link followed from the
   * organisations screen. In component state it would be a filter nobody can
   * send to a colleague.
   */
  it('carries the organisation being narrowed to in the url', () => {
    const match = router.getMatchedRoutes('/platform/deployments')

    expect(match.foundRoute?.options.validateSearch).toBeDefined()
  })
})

const IDENTITY = `${DEPLOYMENT}/identity`

/** The redirect an address answers with, or null when it renders. */
function redirectOf(pathname: string, search: Record<string, unknown> = {}): string | null {
  const { foundRoute, routeParams } = router.getMatchedRoutes(pathname)
  const options = foundRoute?.options as {
    beforeLoad?: (context: { params: Record<string, string>; search: unknown }) => void
    validateSearch?: (search: Record<string, unknown>) => unknown
  }

  try {
    options.beforeLoad?.({
      params: routeParams as Record<string, string>,
      search: options.validateSearch ? options.validateSearch(search) : search,
    })
  } catch (error) {
    return isRedirect(error) ? (error as { options: { to: string } }).options.to : null
  }
  return null
}

describe('the settings pages that became tabs', () => {
  it('puts each tab on its own page', () => {
    expect(shellsFor(`${DEPLOYMENT}/backups`)).toContain('PageBackupsFeature')
    expect(shellsFor(`${DEPLOYMENT}/security`)).toContain('PageNetworkAccessFeature')
  })

  it.each([
    ['/settings/network-access', `${DEPLOYMENT}/security`],
    ['/settings/backups', `${DEPLOYMENT}/backups`],
    ['/settings/branding', `${IDENTITY}/realms/master/branding/universal-login`],
  ])('sends the old address %s to its tab', (from, to) => {
    expect(redirectOf(`${DEPLOYMENT}${from}`)).toBe(to)
  })
})

describe('the Identity space', () => {
  const branding = `${IDENTITY}/realms/acme/branding`

  /**
   * Beside the deployment's shell, not inside it: nested, the page would open
   * with two stacked headers.
   */
  it.each([
    '/branding/universal-login',
    '/branding/universal-login/theme-1',
    '/branding/email-provider',
  ])('puts %s under the Identity shell and the Branding navigation', (path) => {
    const shells = shellsFor(`${IDENTITY}/realms/acme${path}`)

    expect(shells).toContain('IdentityLayout')
    expect(shells).toContain('DeploymentBrandingLayout')
    expect(shells).not.toContain('DeploymentLayout')
  })

  it('opens each Branding page on its own screen', () => {
    expect(shellsFor(`${branding}/universal-login`)).toContain('PageUniversalLoginFeature')
    expect(shellsFor(`${branding}/universal-login`)).not.toContain('PageThemeBuilderFeature')
    expect(shellsFor(`${branding}/universal-login/theme-1`)).toContain('PageThemeBuilderFeature')
    expect(shellsFor(`${branding}/universal-login/theme-1`)).not.toContain('PageUniversalLoginFeature')
    expect(shellsFor(`${branding}/email-provider`)).toContain('PageEmailProviderFeature')
  })

  it('reads the realm, and the theme, from the path', () => {
    const { routeParams } = router.getMatchedRoutes(`${branding}/universal-login/theme-1`)

    expect(routeParams).toMatchObject({ realm: 'acme', themeId: 'theme-1' })
  })

  it('reads the realm from the path and from nothing else', () => {
    const { foundRoute } = router.getMatchedRoutes(`${branding}/universal-login`)

    expect(foundRoute?.options.validateSearch).toBeUndefined()
  })

  it.each(['/webhooks', '/webhooks/new', '/webhooks/w-1', '/webhooks/w-1/edit', '/agents'])(
    'puts %s under the Identity shell and its tab frame, in no realm',
    (path) => {
      const shells = shellsFor(`${IDENTITY}${path}`)

      expect(shells).toContain('IdentityLayout')
      expect(shells).toContain('DeploymentTabLayout')
      expect(shells).not.toContain('DeploymentLayout')
      expect(router.getMatchedRoutes(`${IDENTITY}${path}`).routeParams).not.toHaveProperty('realm')
    },
  )

  it('resolves /webhooks/new to the form rather than to a webhook called new', () => {
    expect(shellsFor(`${IDENTITY}/webhooks/new`)).toContain('PageWebhookFormFeature')
  })

  it('opens the agents screen', () => {
    expect(shellsFor(`${IDENTITY}/agents`)).toContain('PageAgentsFeature')
  })

  /**
   * Without this the bare addresses render a header and an empty body, which
   * looks like a page that failed to load.
   */
  it('sends the bare addresses somewhere rather than nowhere', () => {
    expect(redirectOf(IDENTITY)).toBe(`${IDENTITY}/realms/master`)
    expect(redirectOf(`${IDENTITY}/realms/acme`)).toBe(
      `${IDENTITY}/realms/acme/branding/universal-login`,
    )
    expect(redirectOf(`${IDENTITY}/realms/acme/branding`)).toBe(
      `${IDENTITY}/realms/acme/branding/universal-login`,
    )
  })

  it('leaves the platform section as it was', () => {
    expect(shellsFor('/platform/dataplanes')).toContain('PlatformLayout')
    expect(shellsFor('/platform/dataplanes')).not.toContain('IdentityLayout')
  })
})

describe('the addresses from before Identity', () => {
  it.each([
    ['/branding', {}, `${IDENTITY}/realms/master/branding/universal-login`],
    ['/branding', { realm: 'acme' }, `${IDENTITY}/realms/acme/branding/universal-login`],
    ['/branding/universal-login', { realm: 'acme' }, `${IDENTITY}/realms/acme/branding/universal-login`],
    ['/branding/universal-login/theme-1', { realm: 'acme' }, `${IDENTITY}/realms/acme/branding/universal-login/theme-1`],
    ['/branding/email-provider', { realm: 'acme' }, `${IDENTITY}/realms/acme/branding/email-provider`],
    ['/customization', {}, `${IDENTITY}/realms/master/branding/universal-login`],
    ['/webhooks', {}, `${IDENTITY}/webhooks`],
    ['/webhooks/new', {}, `${IDENTITY}/webhooks/new`],
    ['/webhooks/w-1', {}, `${IDENTITY}/webhooks/w-1`],
    ['/webhooks/w-1/edit', {}, `${IDENTITY}/webhooks/w-1/edit`],
    ['/agents', {}, `${IDENTITY}/agents`],
  ])('sends %s %j to %s', (from, search, to) => {
    expect(redirectOf(`${DEPLOYMENT}${from}`, search)).toBe(to)
  })
})
