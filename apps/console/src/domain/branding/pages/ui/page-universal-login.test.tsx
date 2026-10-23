import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { DELETE_ACTIVE_HINT, PageUniversalLogin } from './page-universal-login'

const noop = () => {}

function render(over: Partial<React.ComponentProps<typeof PageUniversalLogin>> = {}) {
  return renderToStaticMarkup(
    <PageUniversalLogin
      isLoading={false}
      unsupported={false}
      realm='master'
      realms={['master', 'acme']}
      onRealmChange={noop}
      themes={[]}
      state={null}
      planOpen
      canManage
      isBusy={false}
      legacy={null}
      onCreate={noop}
      onImport={noop}
      onImportLegacy={noop}
      onEdit={noop}
      onActivate={noop}
      onDuplicate={noop}
      onExport={noop}
      onDelete={noop}
      onDeactivate={noop}
      {...over}
    />
  )
}

function theme(id: string, name: string, active = false): Schemas.ThemeView {
  return {
    id,
    name,
    tokens: {},
    active,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-02-03T00:00:00Z',
  }
}

const autumn = theme('t1', 'Autumn', true)
const winter = theme('t2', 'Winter')
const legacy: Schemas.Branding = { colors: { primary: '#112233' }, radius: 4 }

function disabled(html: string, label: string): boolean {
  const button = html.match(new RegExp(`<button[^>]*>(?:(?!</button>)[^])*?${label}`))
  if (!button) throw new Error(`no button ${label}`)
  return /<button[^>]*\sdisabled=""/.test(button[0])
}

describe('the Universal login page', () => {
  it('says what the page is for and offers the realms', () => {
    const html = render()

    expect(html).toContain('Universal login')
    expect(html).toContain('Realm')
  })

  it('explains the default look and offers the first theme when there is none', () => {
    const html = render()

    expect(html).toContain('No themes yet')
    expect(html).toContain('Without an active theme FerrisKey uses its default look')
    expect(html).toContain('Create your first theme')
    expect(html).not.toContain('Use FerrisKey’s default look')
  })

  it('lists the themes with the active one badged', () => {
    const html = render({ themes: [autumn, winter] })

    expect(html).toContain('Autumn')
    expect(html).toContain('Winter')
    expect(html).toContain('Active')
    expect(html.match(/>Active</g)).toHaveLength(1)
    expect(html).toContain('Updated Feb 3, 2026')
    expect(html).not.toContain('No themes yet')
  })

  it('offers every action to a manager', () => {
    const html = render({ themes: [autumn, winter] })

    for (const label of [
      'New theme',
      'Import JSON',
      'Edit',
      'Activate',
      'Duplicate',
      'Export JSON',
      'Delete',
    ]) {
      expect(html, label).toContain(label)
    }
    expect(html).toContain('Use FerrisKey’s default look')
  })

  it('does not offer to activate the active theme', () => {
    expect(render({ themes: [autumn] })).not.toContain('>Activate<')
  })

  it('only offers the default look while a theme is active', () => {
    expect(render({ themes: [winter] })).not.toContain('Use FerrisKey’s default look')
  })

  it('disables deleting the active theme and says why', () => {
    const html = render({ themes: [autumn, winter] })

    expect(html).toContain(DELETE_ACTIVE_HINT)
    expect(html).toContain(`title="${DELETE_ACTIVE_HINT}"`)
    expect(html.match(/<button[^>]*disabled=""[^>]*title=/)).not.toBeNull()
  })

  it('says how many themes the realm holds', () => {
    expect(render({ themes: [autumn, winter] })).toContain('2 of 10 themes.')
  })

  it('stops offering new themes at ten', () => {
    const themes = Array.from({ length: 10 }, (_, index) => theme(`t${index}`, `Theme ${index}`))
    const html = render({ themes })

    expect(html).toContain('the most it can')
    expect(disabled(html, 'New theme')).toBe(true)
    expect(disabled(html, 'Import JSON')).toBe(true)
    expect(disabled(html, 'Duplicate')).toBe(true)
    expect(disabled(html, 'Edit')).toBe(false)
  })
})

describe('the state of the realm', () => {
  it('shows nothing while nothing was ever sent', () => {
    expect(render({ themes: [autumn] })).not.toContain('Applied')
  })

  it('shows the active theme applied', () => {
    const html = render({ themes: [autumn], state: { phase: 'Applied', revision: 2 } })

    expect(html).toContain('Active theme: Autumn')
    expect(html).toContain('Applied')
  })

  it('shows a pending state as applying', () => {
    expect(render({ themes: [autumn], state: { phase: 'Pending', revision: 2 } })).toContain(
      'Applying'
    )
  })

  it('shows why it failed', () => {
    const html = render({
      themes: [autumn],
      state: { phase: 'Failed', message: 'FerrisKey refused colors.error', revision: 2 },
    })

    expect(html).toContain('Failed')
    expect(html).toContain('FerrisKey refused colors.error')
  })

  it('names the default look when nothing is active', () => {
    expect(render({ themes: [winter], state: { phase: 'Applied', revision: 3 } })).toContain(
      'FerrisKey’s default look'
    )
  })
})

describe('a viewer', () => {
  it('sees the themes and none of the buttons that change them', () => {
    const html = render({ themes: [autumn, winter], canManage: false })

    expect(html).toContain('Autumn')
    expect(html).toContain('Winter')
    for (const label of [
      'New theme',
      'Import JSON',
      'Activate',
      'Duplicate',
      'Delete',
      'Use FerrisKey’s default look',
    ]) {
      expect(html, label).not.toContain(label)
    }
    expect(html).toContain('Export JSON')
    expect(html).toContain('>View<')
    expect(html).not.toContain('>Edit<')
  })

  it('is told to ask when there is no theme', () => {
    const html = render({ canManage: false })

    expect(html).toContain('No themes yet')
    expect(html).toContain('Ask someone who can manage instances')
    expect(html).not.toContain('Create your first theme')
  })

  it('is not offered the legacy import', () => {
    const html = render({ canManage: false, legacy })

    expect(html).not.toContain('Import your current branding as a theme')
  })
})

describe('a plan without branding', () => {
  it('keeps the list readable and says why nothing can change', () => {
    const html = render({ themes: [autumn, winter], planOpen: false, tier: 'Free' })

    expect(html).toContain('Autumn')
    expect(html).toContain('Branding is not included in your plan')
    expect(html).toContain('Branding is not included in the Free tier of this deployment.')
  })

  it('disables every button that changes something', () => {
    const html = render({ themes: [autumn, winter], planOpen: false })

    for (const label of [
      'New theme',
      'Import JSON',
      'Activate',
      'Duplicate',
      'Delete',
      'Use FerrisKey’s default look',
    ]) {
      expect(disabled(html, label), label).toBe(true)
    }
    expect(disabled(html, 'Edit')).toBe(false)
    expect(disabled(html, 'Export JSON')).toBe(false)
  })

  it('disables the first theme too', () => {
    expect(disabled(render({ planOpen: false }), 'Create your first theme')).toBe(true)
  })

  it('does not say it when the plan has branding', () => {
    expect(render()).not.toContain('not included in your plan')
  })
})

describe('the legacy branding', () => {
  it('offers to import it on master when there is no theme yet', () => {
    const html = render({ legacy })

    expect(html).toContain('Import your current branding as a theme')
    expect(disabled(html, 'Import your current branding as a theme')).toBe(false)
  })

  it('is not offered when the realm already has a theme', () => {
    expect(render({ legacy, themes: [winter] })).not.toContain('Import your current branding')
  })

  it('is not offered on another realm', () => {
    expect(render({ legacy, realm: 'acme' })).not.toContain('Import your current branding')
  })

  it('is not offered when the deployment has none', () => {
    expect(render({ legacy: null })).not.toContain('Import your current branding')
    expect(render({ legacy: { colors: {}, radius: null } })).not.toContain(
      'Import your current branding'
    )
  })

  it('is locked with the plan', () => {
    expect(
      disabled(render({ legacy, planOpen: false }), 'Import your current branding as a theme')
    ).toBe(true)
  })
})

describe('around the list', () => {
  it('says login pages are for FerrisKey deployments', () => {
    const html = render({ unsupported: true })

    expect(html).toContain('Login pages are available on FerrisKey deployments')
    expect(html).not.toContain('New theme')
  })

  it('shows a placeholder while loading', () => {
    const html = render({ isLoading: true })

    expect(html).not.toContain('No themes yet')
    expect(html).toContain('Universal login')
  })

  it('shows a failure to load inline', () => {
    expect(render({ error: 'HTTP 500' })).toMatch(/role="alert"[^>]*>HTTP 500/)
  })

  it('shows what the platform said about an action inline', () => {
    const html = render({
      themes: [autumn],
      actionError: 'name: a theme of this realm already has this name',
    })

    expect(html).toMatch(/role="alert"[^>]*>name: a theme of this realm already has this name/)
  })
})
