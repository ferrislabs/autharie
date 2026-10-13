import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ADMINISTRATOR_COPY, type TenantAdministratorView } from '../../../tenant-realm'
import { TenantAdministratorPanel } from './tenant-administrator-panel'

const noop = () => {}

function render(administrator: TenantAdministratorView, overrides = {}) {
  return renderToStaticMarkup(
    <TenantAdministratorPanel
      administrator={administrator}
      revealing={false}
      onReveal={noop}
      {...overrides}
    />
  )
}

describe('the administrator of a free realm', () => {
  it('offers to reveal the credential once, and warns that it is shown once', () => {
    const html = render({ username: 'admin', credential: 'available' })

    expect(html).toContain(ADMINISTRATOR_COPY.reveal)
    expect(html).toContain(ADMINISTRATOR_COPY.shownOnce)
    expect(html).toContain('admin')
  })

  it('shows the revealed password with a way to copy it', () => {
    const html = render(
      { username: 'admin', credential: 'available' },
      { revealed: { username: 'admin', password: 'Tmp-Pass-42!' } }
    )

    expect(html).toContain('Tmp-Pass-42!')
    expect(html).toContain(ADMINISTRATOR_COPY.copyPassword)
    expect(html).not.toContain(ADMINISTRATOR_COPY.reveal)
  })

  it('offers nothing to reveal once the credential was revealed, expired or is not there yet', () => {
    for (const [credential, says] of [
      ['revealed', ADMINISTRATOR_COPY.revealed],
      ['expired', ADMINISTRATOR_COPY.expired],
      ['pending', ADMINISTRATOR_COPY.pending],
    ] as const) {
      const html = render({ credential })

      expect(html).toContain(says)
      expect(html).not.toContain(ADMINISTRATOR_COPY.reveal)
    }
  })

  it('says why a reveal was refused', () => {
    const html = render(
      { username: 'admin', credential: 'available' },
      { error: 'only an organisation administrator can reveal the realm administrator credential' }
    )

    expect(html).toContain('only an organisation administrator')
  })
})
