import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Dialog } from '@/components/ui/dialog'
import { TheLink } from './invite-dialog'

const props = {
  onDone: () => {},
  link: 'https://console.test/invitations/accept?token=t',
}

describe('the invite dialog link step', () => {
  it('says an account was created when the platform created one', () => {
    const html = renderToStaticMarkup(
      <Dialog>
        <TheLink {...props} createdAccountFor='new@acme.test' />
      </Dialog>,
    )

    expect(html).toContain('An account was created for new@acme.test')
    expect(html).toContain('they will receive an email to set their password.')
  })

  it('stays quiet about accounts when one already existed', () => {
    const html = renderToStaticMarkup(
      <Dialog>
        <TheLink {...props} createdAccountFor={null} />
      </Dialog>,
    )

    expect(html).not.toContain('An account was created')
  })
})
