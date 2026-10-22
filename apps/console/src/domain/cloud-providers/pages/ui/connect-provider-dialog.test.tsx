import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Dialog } from '@/components/ui/dialog'
import { providerById } from '../../providers'
import { CredentialForm, ProviderPicker } from './connect-provider-dialog'
import { stepWidth } from './connect-step'

const noop = () => {}

function form(props: { isSaving?: boolean; refusal?: string } = {}) {
  return renderToStaticMarkup(
    <Dialog open>
      <CredentialForm
        provider='scaleway'
        isSaving={props.isSaving ?? false}
        refusal={props.refusal}
        onBack={noop}
        onSubmit={noop}
      />
    </Dialog>,
  )
}

function picker() {
  return renderToStaticMarkup(
    <Dialog open>
      <ProviderPicker onSelect={noop} />
    </Dialog>,
  )
}

describe('the provider picker', () => {
  it('shows one card per provider, with its logo, name and description', () => {
    const html = picker()
    const scaleway = providerById('scaleway')

    expect(html).toContain('Connect a provider')
    expect(html.match(/<button[^>]*type="button"[^>]*class="flex flex-col/g)).toHaveLength(1)
    expect(html).toContain('aria-label="Scaleway"')
    expect(html).toContain('>Scaleway</span>')
    expect(html).toContain(scaleway.description)
  })

  it('puts the name to the right of its logo, the description under both', () => {
    const html = picker()
    const logo = html.indexOf('aria-label="Scaleway"')
    const name = html.indexOf('>Scaleway</span>')
    const description = html.indexOf(providerById('scaleway').description)
    expect(logo).toBeLessThan(name)
    expect(name).toBeLessThan(description)
    expect(html).toContain('<span class="flex items-center gap-3">')
  })

  it('does not show the form', () => {
    expect(picker()).not.toContain('Fill these information')
  })
})

describe('the size of the modal', () => {
  it('is narrower on the provider cards than on the configuration it turns into', () => {
    expect(stepWidth({ provider: null })).toBe('sm:max-w-lg')
    expect(stepWidth({ provider: 'scaleway' })).toBe('sm:max-w-4xl')
  })
})

describe('the credential form', () => {
  it('names the provider and guides the customer through the two steps', () => {
    const html = form()
    expect(html).toContain('Connect Scaleway')
    expect(html).toContain('give Autharie access to your Scaleway account.')
    expect(html).toContain('Create an API key')
    expect(html).toContain('Fill these information')
    expect(html).toContain('Autharie refuses a key that holds more than needed')
    expect(html.match(/<li class="relative flex gap-4"/g)).toHaveLength(2)
  })

  it('shows each permission set as a card with its name, why it is needed and a note', () => {
    const html = form()
    const sets = providerById('scaleway').permissionSets
    expect(sets).toHaveLength(5)
    for (const set of sets) {
      expect(html).toContain(`>${set.name}</span>`)
      expect(html).toContain(set.description.replace(/'/g, '&#x27;'))
      expect(html).toContain(set.note)
      expect(set.description).not.toBe('')
      expect(set.note).not.toBe('')
    }
    expect(html.match(/<li class="flex"><div class="rounded-lg border bg-card/g)).toHaveLength(5)
  })

  it('puts three cards on a line, two on a narrow screen', () => {
    expect(form()).toContain('<ul class="grid gap-2 sm:grid-cols-2 md:grid-cols-3">')
  })

  it('leaves room between the Back button and the title', () => {
    const html = form()
    const back = html.match(/<button[^>]*>(?:(?!<\/button>).)*Back<\/button>/)![0]
    expect(back).not.toContain('-mb-')
  })

  it('still lets the customer copy the names', () => {
    expect(form()).toContain('Copy names')
  })

  it('asks for the five fields, the secret one masked, the name on its own line', () => {
    const html = form()
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
      'type="password"',
    )
    expect(html.match(/<input/g)).toHaveLength(5)
    expect(html.match(/sm:col-span-2/g)).toHaveLength(1)
  })

  it('keeps Connect disabled until everything is filled', () => {
    expect(form()).toMatch(/<button[^>]*type="submit"[^>]*disabled=""/)
  })

  it('has one Back button, above the title and beside the close button', () => {
    const html = form()
    expect(html.match(/>Back</g)).toHaveLength(1)
    expect(html.indexOf('Back')).toBeLessThan(html.indexOf('Connect Scaleway'))
  })

  it('shows a refusal inline', () => {
    const html = form({ refusal: 'Excess permissions: AllProductsFullAccess' })
    expect(html).toContain('role="alert"')
    expect(html).toContain('Excess permissions: AllProductsFullAccess')
  })
})
