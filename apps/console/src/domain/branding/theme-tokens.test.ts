import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import {
  BORDER_KEYS,
  COLOR_KEYS,
  FONT_STYLE_KEYS,
  LEGACY_THEME_NAME,
  MAX_NAME_LENGTH,
  RANGES,
  SPACING_KEYS,
  TOKEN_PATHS,
  defaultTokens,
  describeNameProblem,
  effectiveTokens,
  firstProblem,
  getAt,
  hasLegacyBranding,
  isDirty,
  isValid,
  legacyToTokens,
  setAt,
  toTokens,
  validateTokens,
  withDefaults,
} from './theme-tokens'

function problemsOf(path: string, value: unknown) {
  return validateTokens(setAt(defaultTokens(), path, value))
}

describe('the default tokens', () => {
  it('are FerrisKey’s own look', () => {
    const defaults = defaultTokens()

    expect(defaults.colors.primaryButton).toBe('#635dff')
    expect(defaults.colors.pageBackground).toBe('#000000')
    expect(defaults.colors.error).toBe('#d03c38')
    expect(defaults.fonts.baseSize).toBe(16)
    expect(defaults.fonts.title).toEqual({ weight: 600, sizePct: 150 })
    expect(defaults.fonts.links).toEqual({ weight: 600, sizePct: 87.5, style: 'normal' })
    expect(defaults.borders.widgetRadius).toBe(5)
    expect(defaults.borders.widgetShadow).toBe('large')
    expect(defaults.spacing).toEqual({ widgetPadding: 24, fieldGap: 16, sectionGap: 24 })
  })

  it('hold every token FerrisKey knows, and nothing else', () => {
    const defaults = defaultTokens()

    expect(Object.keys(defaults.colors).sort()).toEqual([...COLOR_KEYS].sort())
    expect(COLOR_KEYS).toHaveLength(18)
    expect(Object.keys(defaults.borders).sort()).toEqual([...BORDER_KEYS, 'widgetShadow'].sort())
    expect(Object.keys(defaults.spacing).sort()).toEqual([...SPACING_KEYS].sort())
    expect(Object.keys(defaults.fonts).sort()).toEqual(
      ['url', 'baseSize', ...FONT_STYLE_KEYS, 'links'].sort()
    )
  })

  it('are valid', () => {
    expect(validateTokens(defaultTokens())).toEqual({})
    expect(firstProblem(defaultTokens())).toBeNull()
  })

  it('are a fresh copy each time', () => {
    const first = defaultTokens()
    first.colors.error = '#000000'

    expect(defaultTokens().colors.error).toBe('#d03c38')
  })

  it('list every token path once', () => {
    expect(new Set(TOKEN_PATHS).size).toBe(TOKEN_PATHS.length)
    expect(TOKEN_PATHS).toHaveLength(18 + 2 + 5 * 2 + 3 + 9 + 1 + 3)
    for (const path of TOKEN_PATHS) expect(getAt(defaultTokens(), path), path).not.toBeUndefined()
  })
})

describe('withDefaults', () => {
  it('turns nothing into the defaults', () => {
    expect(withDefaults(undefined)).toEqual(defaultTokens())
    expect(withDefaults(null)).toEqual(defaultTokens())
    expect(withDefaults({})).toEqual(defaultTokens())
  })

  it('keeps what is set and fills the rest', () => {
    const tokens = withDefaults({ colors: { primaryButton: '#112233' }, spacing: { fieldGap: 4 } })

    expect(tokens.colors.primaryButton).toBe('#112233')
    expect(tokens.colors.error).toBe('#d03c38')
    expect(tokens.spacing).toEqual({ widgetPadding: 24, fieldGap: 4, sectionGap: 24 })
  })

  it('treats null like absent', () => {
    const tokens = withDefaults({
      colors: { links: null },
      fonts: { baseSize: null, title: null },
      borders: null,
    })

    expect(tokens).toEqual(defaultTokens())
  })

  it('completes a font style that only says half of it', () => {
    const tokens = withDefaults({
      fonts: {
        title: { weight: 800 } as Schemas.ThemeFontStyle,
        links: { style: 'underline' } as Schemas.ThemeLinkFont,
      },
    })

    expect(tokens.fonts.title).toEqual({ weight: 800, sizePct: 150 })
    expect(tokens.fonts.links).toEqual({ weight: 600, sizePct: 87.5, style: 'underline' })
  })

  it('keeps a font url', () => {
    expect(withDefaults({ fonts: { url: 'https://fonts.example.com/a.css' } }).fonts.url).toBe(
      'https://fonts.example.com/a.css'
    )
  })

  it('does not hide an out-of-range value from validation', () => {
    const tokens = withDefaults({ borders: { widgetRadius: 99 } })

    expect(tokens.borders.widgetRadius).toBe(99)
    expect(validateTokens(tokens)['borders.widgetRadius']).toBeDefined()
  })
})

describe('toTokens', () => {
  it('sends every font style complete', () => {
    const { fonts } = toTokens(defaultTokens())

    for (const key of FONT_STYLE_KEYS) {
      expect(fonts?.[key], key).toEqual({
        weight: expect.any(Number),
        sizePct: expect.any(Number),
      })
    }
    expect(fonts?.links).toEqual({ weight: 600, sizePct: 87.5, style: 'normal' })
  })

  it('sends every token', () => {
    const tokens = toTokens(defaultTokens())

    expect(Object.keys(tokens.colors ?? {})).toHaveLength(18)
    expect(Object.keys(tokens.borders ?? {})).toHaveLength(10)
    expect(Object.keys(tokens.spacing ?? {})).toHaveLength(3)
  })

  it('leaves the font url out when there is none', () => {
    expect(toTokens(defaultTokens()).fonts).not.toHaveProperty('url')

    const empty = setAt(defaultTokens(), 'fonts.url', '   ')
    expect(toTokens(empty).fonts).not.toHaveProperty('url')
  })

  it('trims the font url and lowercases the colours', () => {
    let tokens = setAt(defaultTokens(), 'fonts.url', ' https://fonts.example.com/a.css ')
    tokens = setAt(tokens, 'colors.error', ' #AABBCC ')
    const sent = toTokens(tokens)

    expect(sent.fonts?.url).toBe('https://fonts.example.com/a.css')
    expect(sent.colors?.error).toBe('#aabbcc')
  })

  it('round-trips through withDefaults', () => {
    const tokens = setAt(defaultTokens(), 'borders.widgetShadow', 'none')

    expect(withDefaults(toTokens(tokens))).toEqual(tokens)
  })
})

describe('the colours', () => {
  it.each(COLOR_KEYS)('%s must be #rrggbb', (key) => {
    const path = `colors.${key}`

    expect(problemsOf(path, '#a1B2c3')).toEqual({})
    for (const bad of ['', 'red', '#fff', '#12345', '#1234567', 'a1b2c3', '#gggggg', 12]) {
      expect(problemsOf(path, bad)[path], `${path} ${String(bad)}`).toMatch(/#rrggbb/)
    }
  })

  it('tolerates the spaces a paste leaves', () => {
    expect(problemsOf('colors.error', ' #aabbcc ')).toEqual({})
  })
})

describe('the border sizes', () => {
  it.each(BORDER_KEYS)('%s is a whole number from 0 to 64', (key) => {
    const path = `borders.${key}`

    expect(problemsOf(path, 0)).toEqual({})
    expect(problemsOf(path, 64)).toEqual({})
    for (const bad of [-1, 65, 1.5, Number.NaN, Infinity, '3']) {
      expect(problemsOf(path, bad)[path], `${path} ${String(bad)}`).toBe(
        'Must be a whole number between 0 and 64.'
      )
    }
  })

  it('the shadow is none, small or large', () => {
    for (const ok of ['none', 'small', 'large']) {
      expect(problemsOf('borders.widgetShadow', ok)).toEqual({})
    }
    expect(problemsOf('borders.widgetShadow', 'huge')['borders.widgetShadow']).toBe(
      'Must be one of none, small, large.'
    )
  })
})

describe('the spacing', () => {
  it.each(SPACING_KEYS)('%s is a whole number from 0 to 128', (key) => {
    const path = `spacing.${key}`

    expect(problemsOf(path, 0)).toEqual({})
    expect(problemsOf(path, 128)).toEqual({})
    for (const bad of [-1, 129, 0.5, Number.NaN]) {
      expect(problemsOf(path, bad)[path], `${path} ${String(bad)}`).toBe(
        'Must be a whole number between 0 and 128.'
      )
    }
  })
})

describe('the fonts', () => {
  it('the base size is a whole number from 10 to 24', () => {
    expect(problemsOf('fonts.baseSize', 10)).toEqual({})
    expect(problemsOf('fonts.baseSize', 24)).toEqual({})
    for (const bad of [9, 25, 16.5, Number.NaN]) {
      expect(problemsOf('fonts.baseSize', bad)['fonts.baseSize']).toBe(
        'Must be a whole number between 10 and 24.'
      )
    }
  })

  it.each([...FONT_STYLE_KEYS, 'links'])('%s has a weight in steps of 100', (key) => {
    const path = `fonts.${key}.weight`

    for (const ok of [100, 500, 900]) expect(problemsOf(path, ok)).toEqual({})
    for (const bad of [0, 50, 1000, Number.NaN]) {
      expect(problemsOf(path, bad)[path], `${path} ${bad}`).toBe(
        'Must be a whole number between 100 and 900.'
      )
    }
    for (const bad of [150, 450, 899]) {
      expect(problemsOf(path, bad)[path], `${path} ${bad}`).toBe(
        'Must be a weight in steps of 100: 100, 200, ... 900.'
      )
    }
  })

  it.each([...FONT_STYLE_KEYS, 'links'])('%s has a size from 50 to 300 percent', (key) => {
    const path = `fonts.${key}.sizePct`

    for (const ok of [50, 87.5, 300]) expect(problemsOf(path, ok)).toEqual({})
    for (const bad of [49.9, 300.1, Number.NaN, Infinity, '100']) {
      expect(problemsOf(path, bad)[path], `${path} ${String(bad)}`).toBe(
        'Must be a number between 50 and 300.'
      )
    }
  })

  it('a link is normal or underlined', () => {
    expect(problemsOf('fonts.links.style', 'underline')).toEqual({})
    expect(problemsOf('fonts.links.style', 'wavy')['fonts.links.style']).toBe(
      'Must be one of normal, underline.'
    )
  })

  it('the url is optional and must be https', () => {
    expect(problemsOf('fonts.url', null)).toEqual({})
    expect(problemsOf('fonts.url', '')).toEqual({})
    expect(
      problemsOf('fonts.url', 'https://fonts.example.com/a.css?family=Inter:wght@400')
    ).toEqual({})
    for (const bad of [
      'http://fonts.example.com/a.css',
      'fonts.example.com',
      'https://',
      'https://user@fonts.example.com',
      'https://fonts.example.com/a b',
      'https://fonts.example.com/"x"',
      'https://fonts.example.com/(x)',
      'javascript:alert(1)',
    ]) {
      expect(problemsOf('fonts.url', bad)['fonts.url'], bad).toBe('Must be an https URL.')
    }
  })

  it('the url is at most 2048 characters', () => {
    const long = `https://fonts.example.com/${'a'.repeat(2048)}`

    expect(problemsOf('fonts.url', long)['fonts.url']).toBe('Must be at most 2048 characters.')
  })
})

describe('validating a whole set', () => {
  it('names each problem by its path, first one in the message', () => {
    let tokens = setAt(defaultTokens(), 'spacing.fieldGap', 500)
    tokens = setAt(tokens, 'colors.error', 'nope')

    expect(Object.keys(validateTokens(tokens)).sort()).toEqual(['colors.error', 'spacing.fieldGap'])
    expect(firstProblem(tokens)).toMatch(/^colors\.error: Must be a #rrggbb colour/)
  })

  it('swaps what is unusable for the default, for the preview', () => {
    let tokens = setAt(defaultTokens(), 'colors.error', 'nope')
    tokens = setAt(tokens, 'colors.links', '#010203')
    tokens = setAt(tokens, 'borders.widgetRadius', Number.NaN)

    const effective = effectiveTokens(tokens)

    expect(effective.colors.error).toBe('#d03c38')
    expect(effective.colors.links).toBe('#010203')
    expect(effective.borders.widgetRadius).toBe(5)
    expect(validateTokens(effective)).toEqual({})
  })
})

describe('the name', () => {
  it('holds between 1 and 80 characters', () => {
    expect(describeNameProblem('Autumn')).toBeNull()
    expect(describeNameProblem('é'.repeat(MAX_NAME_LENGTH))).toBeNull()
    expect(describeNameProblem('')).toMatch(/between 1 and 80/)
    expect(describeNameProblem('   ')).toMatch(/between 1 and 80/)
    expect(describeNameProblem('é'.repeat(MAX_NAME_LENGTH + 1))).toMatch(/between 1 and 80/)
  })

  it('refuses control characters', () => {
    expect(describeNameProblem('two\nlines')).toMatch(/control/)
  })
})

describe('what makes a theme dirty or valid', () => {
  const stored = { name: 'Autumn', tokens: toTokens(defaultTokens()) }

  it('is clean when nothing moved', () => {
    expect(isDirty({ name: 'Autumn', tokens: defaultTokens() }, stored)).toBe(false)
  })

  it('is clean for a sparse stored theme left as it is', () => {
    const sparse = { name: 'Old', tokens: { colors: { primaryButton: '#635dff' } } }

    expect(isDirty({ name: 'Old', tokens: withDefaults(sparse.tokens) }, sparse)).toBe(false)
  })

  it('is dirty when a token or the name moved', () => {
    expect(
      isDirty({ name: 'Autumn', tokens: setAt(defaultTokens(), 'colors.error', '#000000') }, stored)
    ).toBe(true)
    expect(isDirty({ name: 'Winter', tokens: defaultTokens() }, stored)).toBe(true)
  })

  it('ignores the spaces around the name and the case of a colour', () => {
    const draft = { name: ' Autumn ', tokens: setAt(defaultTokens(), 'colors.error', '#D03C38') }

    expect(isDirty(draft, stored)).toBe(false)
  })

  it('is invalid with a bad name or a bad token', () => {
    expect(isValid({ name: 'Autumn', tokens: defaultTokens() })).toBe(true)
    expect(isValid({ name: '', tokens: defaultTokens() })).toBe(false)
    expect(isValid({ name: 'Autumn', tokens: setAt(defaultTokens(), 'fonts.baseSize', 99) })).toBe(
      false
    )
  })
})

describe('the ranges', () => {
  it('are the ones the platform enforces', () => {
    expect(RANGES.baseSize).toEqual({ min: 10, max: 24, step: 1 })
    expect(RANGES.weight).toEqual({ min: 100, max: 900, step: 100 })
    expect(RANGES.sizePct).toEqual({ min: 50, max: 300, step: 2.5 })
    expect(RANGES.border).toEqual({ min: 0, max: 64, step: 1 })
    expect(RANGES.spacing).toEqual({ min: 0, max: 128, step: 1 })
  })
})

describe('the legacy branding', () => {
  const colors = {
    primary: '#111111',
    primary_text: '#222222',
    links: '#333333',
    page_background: '#444444',
    widget_background: '#555555',
    text: '#666666',
    error: '#777777',
  }

  it('maps each colour to the token the operator writes', () => {
    expect(legacyToTokens({ colors })).toEqual({
      colors: {
        primaryButton: '#111111',
        primaryButtonLabel: '#222222',
        links: '#333333',
        pageBackground: '#444444',
        widgetBackground: '#555555',
        bodyText: '#666666',
        error: '#777777',
      },
    })
  })

  it('maps the radius to the three radii', () => {
    expect(legacyToTokens({ radius: 8 })).toEqual({
      borders: { widgetRadius: 8, buttonRadius: 8, inputRadius: 8 },
    })
  })

  it('keeps a radius of zero', () => {
    expect(legacyToTokens({ radius: 0 })).toEqual({
      borders: { widgetRadius: 0, buttonRadius: 0, inputRadius: 0 },
    })
  })

  it('leaves out what is not set', () => {
    expect(legacyToTokens({ colors: { primary: '#aabbcc', error: null } })).toEqual({
      colors: { primaryButton: '#aabbcc' },
    })
    expect(legacyToTokens({ colors: {} })).toEqual({})
    expect(legacyToTokens(null)).toEqual({})
    expect(legacyToTokens(undefined)).toEqual({})
  })

  it('maps to tokens the platform accepts', () => {
    const tokens = withDefaults(legacyToTokens({ colors, radius: 24 }))

    expect(validateTokens(tokens)).toEqual({})
    expect(tokens.colors.primaryButton).toBe('#111111')
    expect(tokens.borders.inputRadius).toBe(24)
  })

  it('is there when a colour or a radius is set', () => {
    expect(hasLegacyBranding({ colors: { links: '#abcdef' } })).toBe(true)
    expect(hasLegacyBranding({ radius: 0 })).toBe(true)
    expect(hasLegacyBranding({ colors: {}, radius: null })).toBe(false)
    expect(hasLegacyBranding({ colors: { links: null } })).toBe(false)
    expect(hasLegacyBranding(null)).toBe(false)
    expect(hasLegacyBranding(undefined)).toBe(false)
  })

  it('gives the imported theme a name', () => {
    expect(LEGACY_THEME_NAME).toBe('Current branding')
  })
})
