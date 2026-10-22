import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { describeLimit } from '../../limit'
import { PageWebhooks } from './page-webhooks'

const noop = () => {}

function render(over: Partial<React.ComponentProps<typeof PageWebhooks>> = {}) {
  return renderToStaticMarkup(
    <PageWebhooks
      isLoading={false}
      webhooks={[]}
      limit={describeLimit(0, 'Free')}
      canManage
      onCreate={noop}
      onOpen={noop}
      {...over}
    />
  )
}

const webhook = {
  id: 'w1',
  name: 'Audit',
  realm: 'master',
  endpoint: 'https://hooks.example.com/in',
  events: ['user.created', 'user.deleted'],
  state: { phase: 'Pending' },
} as unknown as Schemas.WebhookView

describe('the webhooks list', () => {
  it('teaches what a webhook is for and offers the first one', () => {
    const html = render()
    expect(html).toContain('Create your first webhook')
    expect(html).toContain('signed with HMAC-SHA256 in the x-ferriskey-signature header')
  })

  it('does not offer to create when the member cannot manage', () => {
    const html = render({ canManage: false })
    expect(html).not.toContain('Create your first webhook')
    expect(html).not.toContain('New webhook')
  })

  it('shows host, realm, event count and state of each webhook', () => {
    const html = render({ webhooks: [webhook], limit: describeLimit(1, 'Free') })
    expect(html).toContain('hooks.example.com')
    expect(html).toContain('master')
    expect(html).toContain('2 events')
    expect(html).toContain('Applying')
    expect(html).toContain('1 of 5')
  })

  it('disables creation at the limit and says why', () => {
    const html = render({ webhooks: [webhook], limit: describeLimit(5, 'Free') })
    expect(html).toContain('5 of 5')
    expect(html).toContain('holds 5 webhooks')
    expect(html).toMatch(/<button[^>]*disabled[^>]*>[^]*?New webhook/)
  })
})
