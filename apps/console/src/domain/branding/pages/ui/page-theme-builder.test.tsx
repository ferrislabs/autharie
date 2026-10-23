import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { defaultTokens, setAt, toTokens } from '../../theme-tokens'
import { PageThemeBuilder } from './page-theme-builder'

const noop = () => {}

function render(over: Partial<React.ComponentProps<typeof PageThemeBuilder>> = {}) {
  return renderToStaticMarkup(
    <PageThemeBuilder
      isLoading={false}
      unsupported={false}
      theme={theme()}
      state={null}
      planOpen
      canManage
      isSaving={false}
      onSave={noop}
      onBack={noop}
      onDirtyChange={noop}
      {...over}
    />
  )
}

function theme(over: Partial<Schemas.ThemeView> = {}): Schemas.ThemeView {
  return {
    id: 't1',
    name: 'Autumn',
    tokens: toTokens(defaultTokens()),
    active: false,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-02-03T00:00:00Z',
    ...over,
  }
}

function withTokens(path: string, value: unknown) {
  return toTokens(setAt(defaultTokens(), path, value))
}

function disabled(html: string, label: string): boolean {
  const button = html.match(new RegExp(`<button[^>]*>(?:(?!</button>)[^])*?${label}`))
  if (!button) throw new Error(`no button ${label}`)
  return /<button[^>]*\sdisabled=""/.test(button[0])
}

describe('the preview', () => {
  it('shows the sign-in card', () => {
    const html = render()

    expect(html).toContain('Preview of the sign-in page')
    for (const text of [
      'Welcome back',
      'Email',
      'Password',
      'Sign in',
      'Continue with Google',
      'Sign in with a passkey',
      'Email me a magic link',
      'Forgot your password?',
      'Wrong email or password.',
    ]) {
      expect(html, text).toContain(text)
    }
  })

  it('reflects the colours of the theme', () => {
    const html = render({
      theme: theme({ tokens: withTokens('colors.primaryButton', '#ff0000') }),
    })

    expect(html).toContain('--fk-color-primary-button:#ff0000')
  })

  it('reflects the page background, the radii and the spacing', () => {
    let tokens = setAt(defaultTokens(), 'colors.pageBackground', '#123456')
    tokens = setAt(tokens, 'borders.widgetRadius', 22)
    tokens = setAt(tokens, 'spacing.fieldGap', 31)
    const html = render({ theme: theme({ tokens: toTokens(tokens) }) })

    expect(html).toContain('--fk-color-page-bg:#123456')
    expect(html).toContain('--fk-radius-widget:22px')
    expect(html).toContain('--fk-spacing-field-gap:31px')
  })

  it('reflects a sparse theme with FerrisKey’s defaults for the rest', () => {
    const html = render({ theme: theme({ tokens: { colors: { error: '#abcdef' } } }) })

    expect(html).toContain('--fk-color-error:#abcdef')
    expect(html).toContain('--fk-color-primary-button:#635dff')
  })
})

describe('the builder', () => {
  it('names the theme and lists the panels', () => {
    const html = render()

    expect(html).toContain('value="Autumn"')
    for (const panel of ['Colors', 'Typography', 'Buttons', 'Inputs', 'Widget', 'Page spacing']) {
      expect(html, panel).toContain(panel)
    }
  })

  it('opens on the colors, each with a picker and a hex field', () => {
    const html = render()

    expect(html).toContain('Primary button')
    expect(html).toContain('id="theme-colors-primaryButton"')
    expect(html).toContain('value="#635dff"')
    expect(html).toContain('type="color"')
  })

  it('offers to save, and to save and activate while the theme is not active', () => {
    const html = render()

    expect(html).toContain('>Save<')
    expect(html).toContain('Save and activate')
  })

  it('offers only Save once the theme is active', () => {
    const html = render({ theme: theme({ active: true }) })

    expect(html).toContain('>Save<')
    expect(html).not.toContain('Save and activate')
    expect(html).toContain('>Active<')
    expect(html).toContain('saving it sends it to FerrisKey')
  })

  it('has nothing to save yet, but may be activated as it is', () => {
    const html = render()

    expect(disabled(html, 'Save')).toBe(true)
    expect(disabled(html, 'Save and activate')).toBe(false)
  })

  it('shows Saving while it saves', () => {
    const html = render({ isSaving: true })

    expect(html).toContain('Saving')
    expect(disabled(html, 'Saving')).toBe(true)
    expect(disabled(html, 'Save and activate')).toBe(true)
  })

  it('shows what the platform said about a save inline', () => {
    expect(render({ saveError: 'name: a theme already has this name' })).toMatch(
      /role="alert"[^>]*>name: a theme already has this name/
    )
  })

  it('goes back', () => {
    expect(render()).toContain('Back to themes')
  })

  it('shows the state of the realm on the active theme', () => {
    const html = render({
      theme: theme({ active: true }),
      state: { phase: 'Failed', message: 'FerrisKey refused the tokens', revision: 4 },
    })

    expect(html).toContain('Failed')
    expect(html).toMatch(/role="alert"[^>]*>FerrisKey refused the tokens/)
  })

  it('does not show the state of the realm on a theme that is not active', () => {
    expect(render({ state: { phase: 'Applied', revision: 4 } })).not.toContain('Applied')
  })
})

describe('a theme with a value out of range', () => {
  const broken = theme({ tokens: withTokens('colors.error', 'nope') })

  it('says which value and why, next to it', () => {
    const html = render({ theme: broken })

    expect(html).toContain('aria-invalid="true"')
    expect(html).toContain('Must be a #rrggbb colour, for example #635dff.')
    expect(html).toContain('Fix the highlighted values to save.')
  })

  it('flags the panel that holds it', () => {
    expect(render({ theme: broken })).toContain('has a value to fix')
  })

  it('cannot be saved or activated', () => {
    const html = render({ theme: broken })

    expect(disabled(html, 'Save and activate')).toBe(true)
    expect(disabled(html, 'Save')).toBe(true)
  })

  it('still previews, with the default for the value it cannot use', () => {
    expect(render({ theme: broken })).toContain('--fk-color-error:#d03c38')
  })

  it('does not flag a clean theme', () => {
    const html = render()

    expect(html).not.toContain('has a value to fix')
    expect(html).not.toContain('Fix the highlighted values')
  })
})

describe('a viewer', () => {
  it('sees the values and the preview and cannot change them', () => {
    const html = render({ canManage: false })

    expect(html).toContain('value="Autumn"')
    expect(html).toContain('--fk-color-primary-button:#635dff')
    expect(html).not.toContain('>Save<')
    expect(html).not.toContain('Save and activate')
    expect(html).not.toContain('to defaults')
    expect(html).toContain('readOnly=""')
  })
})

describe('a plan without branding', () => {
  it('explains, and disables saving', () => {
    const html = render({ planOpen: false, tier: 'Free' })

    expect(html).toContain('Branding is not included in your plan')
    expect(html).toContain('Free tier')
    expect(disabled(html, 'Save and activate')).toBe(true)
    expect(html).not.toContain('to defaults')
  })
})

describe('around the builder', () => {
  it('says a theme is missing', () => {
    const html = render({ theme: null })

    expect(html).toContain('Theme not found')
    expect(html).toContain('Back to themes')
  })

  it('says login pages are for FerrisKey deployments', () => {
    expect(render({ unsupported: true })).toContain(
      'Login pages are available on FerrisKey deployments'
    )
  })

  it('shows a placeholder while loading', () => {
    const html = render({ isLoading: true })

    expect(html).not.toContain('Theme not found')
    expect(html).not.toContain('Preview of the sign-in page')
  })

  it('shows a failure to load inline', () => {
    expect(render({ theme: null, error: 'HTTP 500' })).toMatch(/role="alert"[^>]*>HTTP 500/)
  })
})
