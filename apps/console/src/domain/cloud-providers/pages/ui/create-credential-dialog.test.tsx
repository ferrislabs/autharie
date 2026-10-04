import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Dialog } from '@/components/ui/dialog'
import { providerById } from '../../providers'
import { CredentialForm } from './create-credential-dialog'
import { NewCredentialMenu } from './new-credential-menu'

const noop = () => {}

function render(props: { isSaving?: boolean; refusal?: string } = {}) {
  return renderToStaticMarkup(
    <Dialog open>
      <CredentialForm
      provider='scaleway'
      isSaving={props.isSaving ?? false}
      refusal={props.refusal}
      onCancel={noop}
      onSubmit={noop}
      />
    </Dialog>,
  )
}

describe('the new credential button', () => {
  it('is a primary button opening the provider menu', () => {
    const html = renderToStaticMarkup(<NewCredentialMenu onSelect={noop} />)
    expect(html).toContain('New credential')
    expect(html).toContain('aria-haspopup="menu"')
  })
})

describe('the credential form', () => {
  it('guides the customer through the two steps', () => {
    const html = render()
    expect(html).toContain('Create new credential')
    expect(html).toContain('give Autharie access to your Scaleway account.')
    expect(html).toContain('1. Create an API key')
    expect(html).toContain('2. Fill these information')
    expect(html).toContain('Autharie refuses a key that holds more than needed')
  })

  it('lists every permission set', () => {
    const html = render()
    for (const name of providerById('scaleway').permissionSets) {
      expect(html).toContain(`>${name}</li>`)
    }
  })

  it('asks for the five fields, the secret one masked', () => {
    const html = render()
    for (const label of [
      'Name',
      'Access key',
      'Secret access key',
      'Organization id',
      'Project id',
    ]) {
      expect(html).toContain(`>${label}</label>`)
    }
    expect(html.match(/<input[^>]*id="credential-secret_key"[^>]*>/)![0]).toContain(
      'type="password"'
    )
    expect(html.match(/<input/g)).toHaveLength(5)
  })

  it('keeps Create disabled until everything is filled', () => {
    expect(render()).toMatch(/<button[^>]*type="submit"[^>]*disabled=""/)
  })

  it('shows a refusal inline', () => {
    const html = render({ refusal: 'Excess permissions: AllProductsFullAccess' })
    expect(html).toContain('role="alert"')
    expect(html).toContain('Excess permissions: AllProductsFullAccess')
  })
})
