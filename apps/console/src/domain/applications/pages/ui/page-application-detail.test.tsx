import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ApplicationDetail } from '../../types'
import { PageApplicationDetail } from './page-application-detail'
import { SLOW_NOTICE } from './request-notice'

vi.mock('@tanstack/react-router', async () => {
  const { createElement } = await import('react')
  return {
    Link: ({ to, children, title }: { to: string; children: React.ReactNode; title?: string }) =>
      createElement('a', { href: to, title }, children),
  }
})

const noop = () => {}

const detail: ApplicationDetail = {
  id: 'id-web',
  client_id: 'web',
  name: 'Web console',
  enabled: true,
  client_type: 'confidential',
  public_client: false,
  service_account_enabled: true,
  protocol: 'openid-connect',
  registration_source: 'admin',
  created_at: '2026-10-23T10:00:00Z',
  direct_access_grants_enabled: false,
  require_pkce: true,
  token_exchange_enabled: false,
  consent_required: true,
  redirect_uris: ['https://app.example/cb', 'https://app.example/other'],
  access_token_lifetime: 300,
  updated_at: '2026-10-24T10:00:00Z',
}

function render(over: Partial<React.ComponentProps<typeof PageApplicationDetail>> = {}) {
  return renderToStaticMarkup(
    <PageApplicationDetail
      realm='master'
      application={detail}
      isLoading={false}
      isSlow={false}
      backPath='/d/identity/realms/master/applications'
      agentsPath='/d/identity/agents'
      onRetry={noop}
      {...over}
    />,
  )
}

describe('the application detail page', () => {
  it('shows the application, read only', () => {
    const html = render()

    expect(html).toContain('Web console')
    expect(html).toContain('>web<')
    expect(html).toContain('id-web')
    expect(html).toContain('Openid connect')
    expect(html).toContain('Confidential')
    expect(html).toContain('Enabled')
    expect(html).not.toContain('<input')
    expect(html).not.toContain('<button')
  })

  it('lists the redirect URIs', () => {
    const html = render()

    expect(html).toContain('https://app.example/cb')
    expect(html).toContain('https://app.example/other')
  })

  it('says there is no redirect URI when there is none', () => {
    expect(render({ application: { ...detail, redirect_uris: [] } })).toContain('No redirect URI.')
  })

  it('reads the sign-in switches as yes or no', () => {
    const html = render()

    expect(html).toMatch(/Service account<[^]*?Yes/)
    expect(html).toMatch(/PKCE required<[^]*?Yes/)
    expect(html).toMatch(/Token exchange<[^]*?No/)
  })

  it('shows the token lifetimes it has and leaves the others out', () => {
    const html = render()

    expect(html).toContain('Access token')
    expect(html).toContain('300 s')
    expect(html).not.toContain('Refresh token')
    expect(render({ application: { ...detail, access_token_lifetime: undefined } })).not.toContain(
      'Token lifetimes',
    )
  })

  it('goes back to the list', () => {
    expect(render()).toContain('href="/d/identity/realms/master/applications"')
  })

  it('marks an application the Agents tab made', () => {
    const html = render({ application: { ...detail, client_id: 'agent-n8n-ab12cd' } })

    expect(html).toContain('>Agent<')
    expect(html).toContain('href="/d/identity/agents"')
    expect(render()).not.toContain('>Agent<')
  })

  it('shows a loading state while it waits, and says when it is slow', () => {
    expect(render({ application: null, isLoading: true })).toContain('Loading the application')
    const slow = render({ application: null, isLoading: true, isSlow: true })

    expect(slow).toContain(SLOW_NOTICE)
    expect(slow).toContain('href="/d/identity/realms/master/applications"')
  })

  it('shows why it failed, with a way to try again', () => {
    const html = render({ application: null, error: 'The request expired before the answer arrived. Try again.' })

    expect(html).toMatch(/role="alert"[^>]*>[^]*?expired/)
    expect(html).toContain('Try again')
  })

  it('never shows a secret, even if one slipped into the data', () => {
    const html = render({ application: { ...detail, secret: 'hunter2' } as ApplicationDetail })

    expect(html).not.toContain('hunter2')
  })
})
