import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { IamView } from '@/domain/identity/iam/iam-view'
import type { AdminAccess } from './admin-access'
import { AdminAccessCard } from './admin-access-card'

const EP = 'https://acme.example.com'

function render(view: IamView<AdminAccess>) {
  return renderToStaticMarkup(<AdminAccessCard endpoint={EP} view={view} onIssue={() => {}} />)
}

describe('the admin access card', () => {
  it('offers to generate when idle', () => {
    const html = render({ phase: 'idle' })
    expect(html).toContain('Admin access')
    expect(html).toContain('Generate admin access')
  })

  it('says it is asking while pending', () => {
    expect(render({ phase: 'loading', slow: false })).toContain('Asking the deployment')
  })

  it('shows the error and a retry', () => {
    const html = render({ phase: 'failed', message: 'nope' })
    expect(html).toContain('nope')
    expect(html).toContain('Try again')
  })

  it('shows the credentials once, with the secret masked', () => {
    const html = render({
      phase: 'ready',
      result: { clientId: 'autharie-admin', clientSecret: 'topsecret' },
    })
    expect(html).toContain('autharie-admin')
    expect(html).not.toContain('topsecret')
    expect(html).toContain('Shown once. Store it in your secret manager now.')
    expect(html).toContain('/protocol/openid-connect/token')
    expect(html).toContain('$CLIENT_SECRET')
  })
})
