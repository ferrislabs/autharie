import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { ApplicationSummary } from '../../types'
import { PageApplications } from './page-applications'
import { SLOW_NOTICE } from './request-notice'

vi.mock('@tanstack/react-router', async () => {
  const { createElement } = await import('react')
  return {
    Link: ({ to, children, title }: { to: string; children: React.ReactNode; title?: string }) =>
      createElement('a', { href: to, title }, children),
  }
})

const noop = () => {}

const web: ApplicationSummary = {
  id: 'id-web',
  client_id: 'web',
  name: 'Web console',
  enabled: true,
  client_type: 'confidential',
  public_client: false,
  service_account_enabled: false,
  protocol: 'openid-connect',
  registration_source: 'admin',
  created_at: '2026-10-23T10:00:00Z',
}

const agent: ApplicationSummary = {
  ...web,
  id: 'id-agent',
  client_id: 'agent-n8n-ab12cd',
  name: 'n8n',
  enabled: false,
  client_type: 'public',
  registration_source: 'autharie',
}

function render(over: Partial<React.ComponentProps<typeof PageApplications>> = {}) {
  return renderToStaticMarkup(
    <PageApplications
      realm='master'
      applications={[web, agent]}
      isLoading={false}
      isSlow={false}
      search=''
      page={1}
      hasNext={false}
      canGoPrevious={false}
      detailPath={(id) => `/d/identity/realms/master/applications/${id}`}
      agentsPath='/d/identity/agents'
      onSearch={noop}
      onNext={noop}
      onPrevious={noop}
      onRetry={noop}
      {...over}
    />,
  )
}

function buttonOf(html: string, label: string): string {
  const found = html.split('</button>').find((part) => part.includes(label))
  if (found === undefined) throw new Error(`no button labelled ${label}`)
  return found.slice(found.lastIndexOf('<button'))
}

const isDisabled = (html: string, label: string) => buttonOf(html, label).includes('disabled=""')

describe('the applications page', () => {
  it('names the page and the realm', () => {
    const html = render()

    expect(html).toContain('Applications')
    expect(html).toContain('master')
  })

  it('lists a row per application with its name, client id, type, status and source', () => {
    const html = render()

    expect(html).toContain('Web console')
    expect(html).toContain('>web<')
    expect(html).toContain('Confidential')
    expect(html).toContain('Enabled')
    expect(html).toContain('Admin')
    expect(html).toContain('n8n')
    expect(html).toContain('Public')
    expect(html).toContain('Disabled')
    expect(html).toContain('Autharie')
  })

  it('opens a row on its detail page', () => {
    const html = render()

    expect(html).toContain('href="/d/identity/realms/master/applications/id-web"')
    expect(html).toContain('href="/d/identity/realms/master/applications/id-agent"')
  })

  it('marks the clients the Agents tab made, and only those, with a link to that tab', () => {
    const html = render()

    expect(html.match(/>Agent</g)).toHaveLength(1)
    expect(html).toMatch(/<a href="\/d\/identity\/agents"[^>]*>[^]*?Agent/)
    expect(render({ applications: [web] })).not.toContain('>Agent<')
  })

  it('says there is nothing, in a realm with no application', () => {
    const html = render({ applications: [] })

    expect(html).toContain('No applications in this realm')
    expect(html).not.toContain('<table')
  })

  it('says nothing matches a search, and names it', () => {
    const html = render({ applications: [], search: 'ghost' })

    expect(html).toContain('No application matches “ghost”')
  })

  it('escapes what was searched for', () => {
    const html = render({ applications: [], search: '<script>x</script>' })

    expect(html).not.toContain('<script>x')
  })

  it('shows a loading state and no rows while it waits', () => {
    const html = render({ applications: null, isLoading: true })

    expect(html).toContain('Loading applications')
    expect(html).not.toContain('<table')
    expect(html).not.toContain('longer than usual')
    expect(html).not.toContain('No applications in this realm')
  })

  it('says it is taking longer than usual once it is', () => {
    const html = render({ applications: null, isLoading: true, isSlow: true })

    expect(html).toContain(SLOW_NOTICE)
    expect(html).toContain('taking longer than usual')
  })

  it('shows why it failed, with a way to try again', () => {
    const html = render({ applications: null, error: 'realm "ghost" was not found' })

    expect(html).toMatch(/role="alert"[^>]*>[^]*?realm &quot;ghost&quot; was not found/)
    expect(html).toContain('Try again')
    expect(html).not.toContain('No applications in this realm')
  })

  it('offers a search that is submitted with Enter, in a form', () => {
    const html = render({ search: 'web' })

    expect(html).toMatch(/<form[^>]*role="search"/)
    expect(html).toContain('value="web"')
    expect(html).toContain('press Enter')
  })

  it('has no pager for a single page', () => {
    expect(render()).not.toContain('Next page')
  })

  it('offers the next page when the answer named one', () => {
    const html = render({ hasNext: true })

    expect(html).toContain('Page 1')
    expect(isDisabled(html, 'Next page')).toBe(false)
    expect(isDisabled(html, 'Previous')).toBe(true)
  })

  it('offers to go back on a later page', () => {
    const html = render({ page: 2, canGoPrevious: true })

    expect(html).toContain('Page 2')
    expect(isDisabled(html, 'Previous')).toBe(false)
    expect(isDisabled(html, 'Next page')).toBe(true)
  })

  it('does not let somebody page while a page is loading', () => {
    const html = render({ applications: null, isLoading: true, page: 2, canGoPrevious: true })

    expect(isDisabled(html, 'Previous')).toBe(true)
  })

  it('never shows a secret, even if one slipped into the data', () => {
    const html = render({ applications: [{ ...web, secret: 'hunter2' } as ApplicationSummary] })

    expect(html).not.toContain('hunter2')
    expect(html).not.toContain('secret')
  })
})
