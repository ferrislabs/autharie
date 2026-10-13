import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ADMINISTRATOR_COPY, type TenantAdministratorView } from '../../../tenant-realm'
import { TenantAdministratorPanel } from './tenant-administrator-panel'

const noop = () => {}

function render(
  administrator: TenantAdministratorView,
  overrides: Partial<Parameters<typeof TenantAdministratorPanel>[0]> = {}
) {
  return renderToStaticMarkup(
    <TenantAdministratorPanel
      administrator={administrator}
      issuing={false}
      revealing={false}
      onIssue={noop}
      onReveal={noop}
      {...overrides}
    />
  )
}

describe('the administrator of a free realm', () => {
  it('offers to generate the credential the first time', () => {
    const html = render({ credential: 'none', stale: false })

    expect(html).toContain(ADMINISTRATOR_COPY.generate)
    expect(html).not.toContain(ADMINISTRATOR_COPY.reveal)
    expect(html).not.toContain(ADMINISTRATOR_COPY.regenerate)
  })

  it('waits while the credential is being generated, and offers again once it is stale', () => {
    const waiting = render({ credential: 'issuing', stale: false })
    expect(waiting).toContain(ADMINISTRATOR_COPY.issuing)
    expect(waiting).not.toContain(ADMINISTRATOR_COPY.regenerate)

    const stale = render({ credential: 'issuing', stale: true })
    expect(stale).toContain(ADMINISTRATOR_COPY.stale)
    expect(stale).toContain(ADMINISTRATOR_COPY.regenerate)
  })

  it('reveals a ready credential once and warns that it is shown once', () => {
    const html = render({ credential: 'ready', stale: false, username: 'admin' })

    expect(html).toContain(ADMINISTRATOR_COPY.reveal)
    expect(html).toContain(ADMINISTRATOR_COPY.shownOnce)
    expect(html).toContain(ADMINISTRATOR_COPY.regenerate)
  })

  it('shows the revealed password with a way to copy it', () => {
    const html = render(
      { credential: 'ready', stale: false, username: 'admin' },
      { revealed: { username: 'admin', password: 'Tmp-Pass-42!' } }
    )

    expect(html).toContain('Tmp-Pass-42!')
    expect(html).toContain(ADMINISTRATOR_COPY.copyPassword)
    expect(html).not.toContain(ADMINISTRATOR_COPY.reveal)
  })

  it('offers to regenerate once revealed or expired, never to reveal again', () => {
    for (const [credential, says] of [
      ['revealed', ADMINISTRATOR_COPY.revealed],
      ['expired', ADMINISTRATOR_COPY.expired],
    ] as const) {
      const html = render({ credential, stale: false })

      expect(html).toContain(says)
      expect(html).toContain(ADMINISTRATOR_COPY.regenerate)
      expect(html).not.toContain(`>${ADMINISTRATOR_COPY.reveal}<`)
    }
  })

  it('asks for confirmation before replacing the current password', () => {
    const html = render({ credential: 'revealed', stale: false }, { confirming: true })

    expect(html).toContain(ADMINISTRATOR_COPY.regenerateWarning)
    expect(html).toContain(ADMINISTRATOR_COPY.confirmRegenerate)
  })

  it('says why an action was refused', () => {
    const html = render(
      { credential: 'ready', stale: false },
      { error: ADMINISTRATOR_COPY.unreadable }
    )

    expect(html).toContain(ADMINISTRATOR_COPY.unreadable)
  })
})
