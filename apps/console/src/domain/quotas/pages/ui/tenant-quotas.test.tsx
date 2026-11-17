import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { TenantQuotas } from './tenant-quotas'

const policies: Schemas.TenantQuotaView = {
  kind: 'policies',
  description: 'Agent policies on a deployment.',
  enforced: true,
  defaults: [
    { offer: 'free', quota: { type: 'limited', max: 5 } },
    { offer: 'private', quota: { type: 'unlimited' } },
  ],
}

const users: Schemas.TenantQuotaView = {
  kind: 'users',
  description: 'Users of the deployment.',
  enforced: false,
  defaults: [{ offer: 'free', quota: { type: 'unlimited' } }],
}

const render = (props: Partial<Parameters<typeof TenantQuotas>[0]> = {}) =>
  renderToStaticMarkup(
    <TenantQuotas quotas={[policies]} isLoading={false} canChange onChange={vi.fn()} {...props} />
  )

describe('the quotas of an organisation', () => {
  it('lists each kind with its default for every offer', () => {
    const html = render()

    expect(html).toContain('policies')
    expect(html).toContain('free 5')
    expect(html).toContain('private Unlimited')
  })

  it('shows a row without an override as the default', () => {
    const html = render()

    expect(html).toContain('Default')
    expect(html).not.toContain('for every deployment of this organisation</span>')
  })

  it('shows an override and what it applies to', () => {
    const html = render({
      quotas: [{ ...policies, override: { type: 'limited', max: 20 } }],
    })

    expect(html).toContain('Override')
    expect(html).toContain('>20<')
    expect(html).toContain('every deployment of this organisation')
  })

  it('tells apart an unlimited override from no override', () => {
    const html = render({ quotas: [{ ...policies, override: { type: 'unlimited' } }] })

    expect(html).toContain('Override')
    expect(html).toContain('>Unlimited<')
  })

  it('says a kind is measured and never enforced', () => {
    expect(render({ quotas: [users] })).toContain('Measured, never enforced.')
    expect(render()).not.toContain('Measured, never enforced.')
  })

  it('disables every control for an operator without the right', () => {
    const html = render({ canChange: false })

    expect(html.match(/<button[^>]*\sdisabled=""/g)?.length).toBe(3)
    expect(html).toMatch(/<input[^>]*\sdisabled=""/)
  })

  it('cannot reset what is not overridden', () => {
    expect(render()).toMatch(/<button[^>]*\sdisabled=""[^>]*>Reset to default/)
    expect(
      render({ quotas: [{ ...policies, override: { type: 'limited', max: 1 } }] })
    ).not.toMatch(/<button[^>]*\sdisabled=""[^>]*>Reset to default/)
  })

  it('shows the refusal the server gave', () => {
    expect(render({ refusal: 'This needs the act_on_tenant right' })).toContain(
      'This needs the act_on_tenant right'
    )
  })
})
