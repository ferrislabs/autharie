import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Dialog } from '@/components/ui/dialog'
import { permissionSetsOf, providerById } from '../../providers'
import { CredentialForm, ProviderPicker } from './connect-provider-dialog'
import { stepWidth } from './connect-step'

const noop = () => {}

function form(props: { isSaving?: boolean; refusal?: string; initialStep?: number } = {}) {
  return renderToStaticMarkup(
    <Dialog open>
      <CredentialForm
        provider='scaleway'
        isSaving={props.isSaving ?? false}
        refusal={props.refusal}
        initialStep={props.initialStep}
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
    expect(picker()).not.toContain('Create an application')
  })
})

describe('the size of the modal', () => {
  it('is narrower on the provider cards than on the configuration it turns into', () => {
    expect(stepWidth({ provider: null })).toBe('sm:max-w-lg')
    expect(stepWidth({ provider: 'scaleway' })).toBe('sm:max-w-4xl')
  })
})

describe('the stepper', () => {
  it('shows the four steps, in order, at the top of the form', () => {
    const html = form()
    const at = (label: string) => html.indexOf(`>${label}</span>`)
    expect(html).toContain('aria-label="Steps"')
    expect(at('Application')).toBeGreaterThan(0)
    expect(at('Application')).toBeLessThan(at('Permissions'))
    expect(at('Permissions')).toBeLessThan(at('API key'))
    expect(at('API key')).toBeLessThan(at('Connect'))
    expect(html.indexOf('aria-label="Steps"')).toBeLessThan(html.indexOf('Create an application'))
  })

  it('marks the current step, and only that one', () => {
    expect(form().match(/aria-current="step"/g)).toHaveLength(1)
    expect(form({ initialStep: 2 }).match(/aria-current="step"/g)).toHaveLength(1)
  })

  it('lets the customer go back to a step already done, not ahead to one still to do', () => {
    const html = form({ initialStep: 2 })
    const stepper = html.slice(html.indexOf('aria-label="Steps"'), html.indexOf('Generate an API key'))
    expect(stepper.match(/<button/g)).toHaveLength(4)
    expect(stepper.match(/<button[^>]*disabled=""/g)).toHaveLength(1)
  })

  it('shows no step as still to do on the first step but the three after it', () => {
    const stepper = form().slice(form().indexOf('aria-label="Steps"'), form().indexOf('Create an application'))
    expect(stepper.match(/<button[^>]*disabled=""/g)).toHaveLength(3)
  })
})

describe('the first step', () => {
  it('tells how to create an application, with the console link', () => {
    const html = form()
    expect(html).toContain('Connect Scaleway')
    expect(html).toContain('give Autharie access to your Scaleway account.')
    expect(html).toContain('Create an application')
    expect(html).toContain('<strong class="font-medium text-foreground">IAM &amp; API keys</strong>')
    expect(html).toContain('href="https://console.scaleway.com/iam/applications/create"')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noreferrer"')
    expect(html).toContain('Until a policy is attached, an application has no permission at all.')
  })

  it('goes forward with Next and has no field and no Connect yet', () => {
    const html = form()
    expect(html).toContain('Next')
    expect(html).not.toContain('<input')
    expect(html).not.toMatch(/<button[^>]*type="submit"/)
  })

  it('has a Back button, above the title, that returns to the providers', () => {
    const html = form()
    expect(html.match(/>Back</g)).toHaveLength(1)
    expect(html.indexOf('Back')).toBeLessThan(html.indexOf('Connect Scaleway'))
  })
})

describe('the permissions step', () => {
  const html = () => form({ initialStep: 1 })

  it('explains the policy with two rules, and the menus to use', () => {
    expect(html()).toContain('Grant the permissions')
    expect(html()).toContain('Create one policy with two rules')
    expect(html()).toContain('<strong class="font-medium text-foreground">Resource management</strong>')
    expect(html()).toContain('<strong class="font-medium text-foreground">Organization management</strong>')
    expect(html()).toContain('<strong class="font-medium text-foreground">Specific projects</strong>')
  })

  it('groups the permission sets by rule, with the menu and the scope of each', () => {
    const page = html()
    expect(page.indexOf('Rule 1')).toBeLessThan(page.indexOf('Rule 2'))
    expect(page).toContain('· Resource management')
    expect(page).toContain('· Organization management')
    expect(page).toContain('Specific projects: the project your clusters are created in.')
    expect(page).toContain('These permission sets apply to the whole Organization.')
  })

  it('shows each permission set as a card with its name, why it is needed and a note', () => {
    const page = html()
    const sets = permissionSetsOf(providerById('scaleway'))
    expect(sets).toHaveLength(5)
    for (const set of sets) {
      expect(page).toContain(`>${set.name}</span>`)
      expect(page).toContain(set.description.replace(/'/g, '&#x27;'))
      expect(page).toContain(set.note)
    }
    expect(page.match(/<li class="flex"><div class="rounded-lg border bg-card/g)).toHaveLength(5)
  })

  it('puts three cards on a line, two on a narrow screen', () => {
    expect(html().match(/<ul class="grid gap-2 sm:grid-cols-2 md:grid-cols-3">/g)).toHaveLength(2)
  })

  it('lets the customer copy the names of each rule', () => {
    expect(html().match(/Copy names/g)).toHaveLength(2)
  })

  it('warns not to grant more', () => {
    expect(html()).toContain('Autharie refuses a key that holds more than needed.')
  })
})

describe('the API key step', () => {
  it('tells how to generate the key and that the secret is shown once', () => {
    const html = form({ initialStep: 2 })
    expect(html).toContain('Generate an API key')
    expect(html).toContain('<strong class="font-medium text-foreground">bearer</strong>')
    expect(html).toContain('The secret key is shown only once.')
    expect(html).not.toContain('<input')
  })
})

describe('the last step', () => {
  const html = (props: { isSaving?: boolean; refusal?: string } = {}) =>
    form({ initialStep: 3, ...props })

  it('asks for the five fields, the secret one masked, the name on its own line', () => {
    const page = html()
    for (const label of [
      'Name',
      'Access key',
      'Secret access key',
      'Organization id',
      'Project id',
    ]) {
      expect(page).toContain(`>${label}</label>`)
    }
    expect(page.match(/<input[^>]*id="credential-secret_key"[^>]*>/)![0]).toContain(
      'type="password"',
    )
    expect(page.match(/<input/g)).toHaveLength(5)
    expect(page.match(/sm:col-span-2/g)).toHaveLength(1)
  })

  it('says where to find each value, with a link for the two ids', () => {
    const page = html()
    expect(page).toContain('Shown once, when you generate the API key.')
    expect(page).toContain('<strong class="font-medium text-foreground">Copy ID</strong>')
    expect(page).toContain('href="https://console.scaleway.com/settings/organization/information"')
    expect(page).toContain('href="https://console.scaleway.com/project/settings"')
    expect(page).toContain('The default project has the same id as your Organization.')
  })

  it('has Connect, disabled until everything is filled, and no Next', () => {
    const page = html()
    expect(page).toMatch(/<button[^>]*type="submit"[^>]*disabled=""/)
    expect(page).not.toContain('Next')
  })

  it('shows a refusal inline', () => {
    const page = html({ refusal: 'Excess permissions: AllProductsFullAccess' })
    expect(page).toContain('role="alert"')
    expect(page).toContain('Excess permissions: AllProductsFullAccess')
  })

  it('shows a refusal only on this step', () => {
    expect(form({ refusal: 'Excess permissions: X' })).not.toContain('Excess permissions: X')
  })
})
