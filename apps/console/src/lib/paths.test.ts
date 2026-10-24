import { describe, expect, it } from 'vitest'
import {
  belongsToNoOrganisation,
  identityAddressFor,
  isUnder,
  movedSettingsPath,
  swapRealm,
} from './paths'

const DEPLOYMENT = '/organisations/org-1/deployments/dep-1'

describe('isUnder', () => {
  it('lights up on its own page', () => {
    expect(isUnder(DEPLOYMENT, DEPLOYMENT)).toBe(true)
  })

  it('lights up on a page beneath it', () => {
    expect(isUnder(`${DEPLOYMENT}/logs`, DEPLOYMENT)).toBe(true)
  })

  /**
   * The reason this is a function rather than a `startsWith` at the call
   * site: a prefix that stops mid segment points at a different page.
   */
  it('does not light up on a path that merely starts the same way', () => {
    expect(isUnder('/organisations/org-1/deployments-archive', '/organisations/org-1/deployments')).toBe(false)
  })

  /**
   * The overview of a deployment is the deployment's own path, so without
   * this it would stay lit on every page under it.
   */
  it('an exact entry only lights up on itself', () => {
    expect(isUnder(DEPLOYMENT, DEPLOYMENT, true)).toBe(true)
    expect(isUnder(`${DEPLOYMENT}/logs`, DEPLOYMENT, true)).toBe(false)
  })

  it('ignores a trailing slash on either side', () => {
    expect(isUnder(`${DEPLOYMENT}/`, DEPLOYMENT, true)).toBe(true)
    expect(isUnder(DEPLOYMENT, `${DEPLOYMENT}/`, true)).toBe(true)
  })

  it('does not light up on a sibling', () => {
    expect(isUnder(`${DEPLOYMENT}/usage`, `${DEPLOYMENT}/logs`)).toBe(false)
  })
})

describe('belongsToNoOrganisation', () => {
  /**
   * The bug this exists to close: the console sends anyone with no
   * organisation in the URL back to their own, so reaching the platform
   * bounced in the same frame and the link looked like a dead button.
   */
  it('knows the platform stands outside any organisation', () => {
    expect(belongsToNoOrganisation('/platform')).toBe(true)
    expect(belongsToNoOrganisation('/platform/dataplanes')).toBe(true)
    expect(belongsToNoOrganisation('/platform/releases')).toBe(true)
  })

  it('knows creating an organisation happens before there is one', () => {
    expect(belongsToNoOrganisation('/organisations/create')).toBe(true)
  })

  /**
   * Everything else does belong to one, including an organisation whose id
   * happens to start the same way as the create route.
   */
  it('sends everything else back to an organisation', () => {
    expect(belongsToNoOrganisation('/organisations/org-1')).toBe(false)
    expect(belongsToNoOrganisation('/organisations/org-1/deployments')).toBe(false)
    expect(belongsToNoOrganisation('/organisations/created-org')).toBe(false)
    expect(belongsToNoOrganisation('/')).toBe(false)
  })
})

describe('an invitation link', () => {
  /**
   * The page that puts somebody in an organisation cannot be behind the
   * redirect that sends them to one they are already in. Somebody with no
   * organisation would be asked to create one instead of joining the one
   * they were invited to; somebody with one would never reach the link.
   */
  it('escapes the redirect that sends people to an organisation', () => {
    expect(belongsToNoOrganisation('/invitations/accept')).toBe(true)
  })

  it('does not swallow anything else beginning the same way', () => {
    expect(belongsToNoOrganisation('/invitations')).toBe(false)
    expect(belongsToNoOrganisation('/invitations/accepted-elsewhere')).toBe(false)
  })
})

describe('movedSettingsPath', () => {
  it('names the tab each settings page became', () => {
    expect(movedSettingsPath('org-1', 'dep-1', 'network-access')).toBe(`${DEPLOYMENT}/security`)
    expect(movedSettingsPath('org-1', 'dep-1', 'branding')).toBe(
      `${DEPLOYMENT}/identity/realms/master/branding/universal-login`
    )
    expect(movedSettingsPath('org-1', 'dep-1', 'backups')).toBe(`${DEPLOYMENT}/backups`)
  })
})

describe('identityAddressFor', () => {
  const identity = `${DEPLOYMENT}/identity`

  it.each([
    ['branding', '', 'master', `${identity}/realms/master/branding/universal-login`],
    ['branding', '/universal-login', 'acme', `${identity}/realms/acme/branding/universal-login`],
    ['branding', '/universal-login/theme-1', 'acme', `${identity}/realms/acme/branding/universal-login/theme-1`],
    ['branding', '/email-provider', 'acme', `${identity}/realms/acme/branding/email-provider`],
    ['customization', '', 'master', `${identity}/realms/master/branding/universal-login`],
    ['webhooks', '', 'master', `${identity}/webhooks`],
    ['webhooks', '/new', 'master', `${identity}/webhooks/new`],
    ['webhooks', '/wh-1/edit', 'master', `${identity}/webhooks/wh-1/edit`],
    ['agents', '', 'master', `${identity}/agents`],
  ] as const)('moves %s%s of realm %s to its Identity address', (section, rest, realm, expected) => {
    expect(identityAddressFor(DEPLOYMENT, section, rest, realm)).toBe(expected)
  })
})

describe('swapRealm', () => {
  const page = (realm: string) => `${DEPLOYMENT}/identity/realms/${realm}/branding/email-provider`

  it('keeps the section and changes only the realm', () => {
    expect(swapRealm(page('a'), 'b')).toBe(page('b'))
  })

  it('keeps what follows the section too', () => {
    expect(swapRealm(`${DEPLOYMENT}/identity/realms/a/branding/universal-login/theme-1`, 'b')).toBe(
      `${DEPLOYMENT}/identity/realms/b/branding/universal-login/theme-1`
    )
  })

  it('leaves an address that has no realm alone', () => {
    expect(swapRealm(`${DEPLOYMENT}/identity/agents`, 'b')).toBe(`${DEPLOYMENT}/identity/agents`)
  })

  it('escapes a realm name rather than splitting the path', () => {
    expect(swapRealm(page('a'), 'x/y')).toBe(page('x%2Fy'))
  })
})
