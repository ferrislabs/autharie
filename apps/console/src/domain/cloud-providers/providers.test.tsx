import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PROVIDERS, permissionSetsOf, providerById } from './providers'

describe('the providers a credential can be created for', () => {
  it('offers Scaleway, drawn with its own logo', () => {
    const scaleway = providerById('scaleway')
    const Logo = scaleway.icon
    const markup = renderToStaticMarkup(<Logo className='h-4 w-4' />)

    expect(PROVIDERS.map((provider) => provider.id)).toEqual(['scaleway'])
    expect(scaleway.description).not.toBe('')
    expect(markup).toContain('aria-label="Scaleway"')
    expect(markup).toContain('viewBox="0 0 24 24"')
    expect(markup).toContain('<path d="')
    expect(markup).toContain('h-4 w-4')
  })
})

describe('the guide to connect a Scaleway account', () => {
  const scaleway = providerById('scaleway')

  it('asks for exactly the permission sets the platform verifies, none more', () => {
    expect(permissionSetsOf(scaleway).map((set) => set.name).sort()).toEqual([
      'IAMReadOnly',
      'InstancesReadOnly',
      'KubernetesFullAccess',
      'PrivateNetworksFullAccess',
      'ProjectReadOnly',
    ])
  })

  it('splits them in two rules, as Scaleway scopes some by Organization and the rest by project', () => {
    const [resources, organization] = scaleway.permissionGroups
    expect(resources.menu).toBe('Resource management')
    expect(resources.sets.map((set) => set.name)).toEqual([
      'KubernetesFullAccess',
      'InstancesReadOnly',
      'PrivateNetworksFullAccess',
    ])
    expect(organization.menu).toBe('Organization management')
    expect(organization.sets.map((set) => set.name)).toEqual(['ProjectReadOnly', 'IAMReadOnly'])
  })

  it('says why each permission set is needed, and adds a note', () => {
    for (const set of permissionSetsOf(scaleway)) {
      expect(set.description).toMatch(/^Autharie /)
      expect(set.note).not.toBe('')
    }
  })

  it('has three steps to follow, then the one that connects', () => {
    expect(scaleway.guide.map((step) => step.title)).toEqual(['Application', 'Permissions', 'API key'])
    expect(scaleway.connect.title).toBe('Connect')
    for (const step of scaleway.guide) {
      expect(step.instructions.length).toBeGreaterThan(0)
    }
  })

  it('shows the permission cards on one step only', () => {
    expect(scaleway.guide.filter((step) => step.permissions)).toHaveLength(1)
  })

  it('only links to the Scaleway console', () => {
    const links = [
      ...scaleway.guide.flatMap((step) => (step.link ? [step.link] : [])),
      ...scaleway.fields.flatMap((field) => (field.link ? [field.link] : [])),
    ]
    expect(links.length).toBeGreaterThan(0)
    for (const link of links) {
      expect(link.href.startsWith('https://console.scaleway.com/')).toBe(true)
    }
  })

  it('says where to find each field', () => {
    for (const field of scaleway.fields) {
      expect(field.hint).toBeTruthy()
    }
  })
})
