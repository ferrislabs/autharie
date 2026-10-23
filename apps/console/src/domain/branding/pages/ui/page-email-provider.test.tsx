import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { PageEmailProvider } from './page-email-provider'

const noop = () => {}

function render(over: Partial<React.ComponentProps<typeof PageEmailProvider>> = {}) {
  return renderToStaticMarkup(
    <PageEmailProvider
      isLoading={false}
      unsupported={false}
      realm='master'
      realms={['master', 'acme']}
      onRealmChange={noop}
      smtp={null}
      canManage
      isSaving={false}
      isRemoving={false}
      onSave={noop}
      onRemove={noop}
      {...over}
    />
  )
}

const stored: Schemas.SmtpView = {
  host: 'smtp.example.com',
  port: 465,
  encryption: 'tls',
  username: 'mailer',
  from_email: 'no-reply@example.com',
  from_name: 'Acme',
  password_set: true,
  state: { phase: 'Applied' },
  updated_at: '2026-01-01T00:00:00Z',
}

describe('the email provider page', () => {
  it('says what the page is for', () => {
    const html = render()

    expect(html).toContain('Email provider')
    expect(html).toContain('SMTP server FerrisKey sends this realm')
  })

  it('offers the realms to choose from', () => {
    expect(render()).toContain('Realm')
  })

  it('explains that without a provider no email is sent and offers to set one up', () => {
    const html = render()

    expect(html).toContain('No email provider')
    expect(html).toContain('sends no email')
    expect(html).toContain('Set up an email provider')
    expect(html).not.toContain('id="smtp-host"')
  })

  it('does not offer to set one up when the member cannot manage', () => {
    const html = render({ canManage: false })

    expect(html).toContain('No email provider')
    expect(html).not.toContain('Set up an email provider')
  })

  it('shows the stored settings and that a password is set', () => {
    const html = render({ smtp: stored })

    expect(html).toContain('value="smtp.example.com"')
    expect(html).toContain('value="465"')
    expect(html).toContain('value="mailer"')
    expect(html).toContain('value="Acme"')
    expect(html).toContain('value="no-reply@example.com"')
    expect(html).toContain('A password is set — leave empty to keep it')
    expect(html).not.toContain('hunter2')
    expect(html).toContain('Applied')
  })

  it('says FerrisKey cannot send a test email', () => {
    expect(render({ smtp: stored })).toContain('cannot send a test email')
  })

  it('offers to save and to remove to a member who can manage', () => {
    const html = render({ smtp: stored })

    expect(html).toMatch(/<button[^>]*disabled[^>]*>[^]*?Save/)
    expect(html).toContain('Remove provider')
  })

  it('shows why a provider failed', () => {
    const html = render({
      smtp: { ...stored, state: { phase: 'Failed', message: 'realm not found' } },
    })

    expect(html).toContain('Failed')
    expect(html).toContain('realm not found')
  })

  it('shows a pending provider as applying', () => {
    expect(render({ smtp: { ...stored, state: { phase: 'Pending' } } })).toContain('Applying')
  })

  it('shows the values and no buttons to a viewer', () => {
    const html = render({ smtp: stored, canManage: false })

    expect(html).toContain('value="smtp.example.com"')
    expect(html).not.toContain('Save')
    expect(html).not.toContain('Remove provider')
  })

  it('shows what the platform said inline', () => {
    const html = render({ smtp: stored, saveError: 'the port must be between 1 and 65535' })

    expect(html).toMatch(/role="alert"[^>]*>the port must be between 1 and 65535/)
  })

  it('shows a failure to load inline', () => {
    expect(render({ error: 'HTTP 500' })).toMatch(/role="alert"[^>]*>HTTP 500/)
  })

  it('says email settings are for FerrisKey deployments', () => {
    const html = render({ unsupported: true })

    expect(html).toContain('Email settings are available on FerrisKey deployments')
    expect(html).not.toContain('Set up an email provider')
  })

  it('shows a placeholder while loading', () => {
    const html = render({ isLoading: true })

    expect(html).not.toContain('No email provider')
  })
})
