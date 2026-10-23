import { describe, expect, it } from 'vitest'
import {
  EXPORT_MARKER,
  MAX_FILE_BYTES,
  MAX_THEME_BYTES,
  exportFileName,
  exportTheme,
  parseImport,
} from './theme-files'
import { defaultTokens, setAt, toTokens, withDefaults } from './theme-tokens'

const tokens = toTokens(setAt(defaultTokens(), 'colors.primaryButton', '#112233'))

function accepted(text: string) {
  const result = parseImport(text)
  if (!result.ok) throw new Error(result.error)
  return result.theme
}

function refused(text: string): string {
  const result = parseImport(text)
  if (result.ok) throw new Error('accepted')
  return result.error
}

describe('exporting a theme', () => {
  it('writes the marker, the version, the name and the tokens', () => {
    const file = JSON.parse(exportTheme({ name: 'Autumn', tokens }))

    expect(file).toEqual({ autharie_theme: 1, name: 'Autumn', tokens })
    expect(EXPORT_MARKER).toBe('autharie_theme')
  })

  it('completes a sparse theme so the file stands alone', () => {
    const file = JSON.parse(exportTheme({ name: 'Old', tokens: { colors: { links: '#abcdef' } } }))

    expect(file.tokens.colors.links).toBe('#abcdef')
    expect(file.tokens.fonts.title).toEqual({ weight: 600, sizePct: 150 })
    expect(file.tokens.borders.widgetShadow).toBe('large')
  })

  it('is indented to be read and edited', () => {
    expect(exportTheme({ name: 'Autumn', tokens })).toContain('\n  "name": "Autumn"')
  })

  it('names the file after the theme', () => {
    expect(exportFileName('Autumn')).toBe('autumn.json')
    expect(exportFileName('My  big/new Théme!')).toBe('my-big-new-th-me.json')
    expect(exportFileName('***')).toBe('theme.json')
  })
})

describe('importing an Autharie file', () => {
  it('round-trips an export', () => {
    const theme = accepted(exportTheme({ name: 'Autumn', tokens }))

    expect(theme.name).toBe('Autumn')
    expect(theme.tokens).toEqual(tokens)
  })

  it('completes a sparse set of tokens', () => {
    const theme = accepted(
      JSON.stringify({
        autharie_theme: 1,
        name: 'Sparse',
        tokens: { colors: { error: '#010203' } },
      })
    )

    expect(withDefaults(theme.tokens).colors.error).toBe('#010203')
    expect(theme.tokens.fonts?.title).toEqual({ weight: 600, sizePct: 150 })
  })

  it('drops tokens it does not know', () => {
    const theme = accepted(
      JSON.stringify({
        autharie_theme: 1,
        name: 'Extra',
        tokens: { colors: { neon: '#ffffff', error: '#010203' }, glow: { a: 1 } },
      })
    )

    expect(theme.tokens.colors).not.toHaveProperty('neon')
    expect(theme.tokens).not.toHaveProperty('glow')
    expect(theme.tokens.colors?.error).toBe('#010203')
  })

  it('refuses a version it does not know', () => {
    expect(refused(JSON.stringify({ autharie_theme: 2, name: 'x', tokens: {} }))).toBe(
      'This theme file is version 2; this console reads version 1.'
    )
  })

  it('refuses a file with no tokens', () => {
    expect(refused(JSON.stringify({ autharie_theme: 1, name: 'x' }))).toBe(
      'The file has no tokens.'
    )
  })

  it('takes a name from an empty one and cuts a long one', () => {
    expect(accepted(JSON.stringify({ autharie_theme: 1, name: '  ', tokens: {} })).name).toBe(
      'Imported theme'
    )
    expect(accepted(JSON.stringify({ autharie_theme: 1, tokens: {} })).name).toBe('Imported theme')
    expect(
      Array.from(
        accepted(JSON.stringify({ autharie_theme: 1, name: 'é'.repeat(200), tokens: {} })).name
      )
    ).toHaveLength(80)
    expect(accepted(JSON.stringify({ autharie_theme: 1, name: 'a\nb', tokens: {} })).name).toBe(
      'a b'
    )
  })
})

describe('importing a FerrisKey export', () => {
  const envelope = {
    ferriskey: 'portal-theme',
    version: 1,
    name: 'From FerrisKey',
    config: toTokens(setAt(defaultTokens(), 'borders.widgetRadius', 12)),
    pages: { login: [{ type: 'form', children: [] }] },
    layout: { header: [] },
  }

  it('takes the config and leaves the pages behind', () => {
    const theme = accepted(JSON.stringify(envelope))

    expect(theme.name).toBe('From FerrisKey')
    expect(theme.tokens.borders?.widgetRadius).toBe(12)
    expect(JSON.stringify(theme)).not.toContain('pages')
    expect(JSON.stringify(theme)).not.toContain('layout')
  })

  it('refuses an export with no config', () => {
    const { config, ...rest } = envelope
    void config

    expect(refused(JSON.stringify(rest))).toBe('The FerrisKey export has no config.')
  })

  it('does not mind how large the pages are', () => {
    const big = { ...envelope, pages: { login: 'x'.repeat(MAX_THEME_BYTES * 2) } }

    expect(accepted(JSON.stringify(big)).name).toBe('From FerrisKey')
  })
})

describe('refusing what is not a theme', () => {
  it('says when it is not JSON', () => {
    expect(refused('not json')).toBe('This is not valid JSON.')
    expect(refused('')).toBe('This is not valid JSON.')
  })

  it('says when it is not an object', () => {
    expect(refused('[]')).toBe('Expected a JSON object.')
    expect(refused('"x"')).toBe('Expected a JSON object.')
    expect(refused('null')).toBe('Expected a JSON object.')
  })

  it('says when it is neither format', () => {
    expect(refused('{"name":"x","tokens":{}}')).toBe(
      'This file is neither an Autharie theme nor a FerrisKey export.'
    )
  })

  it('names the token that is out of range', () => {
    const text = JSON.stringify({
      autharie_theme: 1,
      name: 'Bad',
      tokens: { borders: { widgetRadius: 99 } },
    })

    expect(refused(text)).toBe('borders.widgetRadius: Must be a whole number between 0 and 64.')
  })

  it('names a section that is not an object', () => {
    expect(
      refused(JSON.stringify({ autharie_theme: 1, name: 'Bad', tokens: { colors: 'red' } }))
    ).toBe('colors: must be a JSON object.')
    expect(
      refused(JSON.stringify({ autharie_theme: 1, name: 'Bad', tokens: { fonts: { title: 3 } } }))
    ).toBe('fonts.title: must be a JSON object.')
    expect(refused(JSON.stringify({ autharie_theme: 1, name: 'Bad', tokens: [] }))).toBe(
      'tokens: must be a JSON object.'
    )
  })

  it('refuses a value of the wrong type', () => {
    const text = JSON.stringify({
      autharie_theme: 1,
      name: 'Bad',
      tokens: { colors: { error: 12 } },
    })

    expect(refused(text)).toBe('colors.error: Must be a #rrggbb colour, for example #635dff.')
  })

  it('refuses tokens above 32 KiB', () => {
    const text = JSON.stringify({
      autharie_theme: 1,
      name: 'Big',
      tokens: { fonts: { url: `https://fonts.example.com/${'a'.repeat(MAX_THEME_BYTES)}` } },
    })

    expect(refused(text)).toMatch(/^The tokens are \d+ bytes, more than the 32768 allowed\.$/)
  })

  it('refuses a file above the size it will even read', () => {
    expect(refused(' '.repeat(MAX_FILE_BYTES + 1))).toMatch(/larger than/)
  })

  it('counts bytes, not characters', () => {
    const text = JSON.stringify({
      autharie_theme: 1,
      name: 'Big',
      tokens: { fonts: { url: `https://fonts.example.com/${'é'.repeat(MAX_THEME_BYTES / 2)}` } },
    })

    expect(refused(text)).toMatch(/more than the 32768 allowed/)
  })
})
