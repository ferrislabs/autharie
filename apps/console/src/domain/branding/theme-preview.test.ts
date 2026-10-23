import { describe, expect, it } from 'vitest'
import { previewVars } from './theme-preview'
import { defaultTokens, setAt } from './theme-tokens'

describe('the preview variables', () => {
  it('carry the colours', () => {
    const vars = previewVars(setAt(defaultTokens(), 'colors.primaryButton', '#ff0000'))

    expect(vars['--fk-color-primary-button']).toBe('#ff0000')
    expect(vars['--fk-color-page-bg']).toBe('#000000')
    expect(vars['--fk-color-widget-bg']).toBe('#ffffff')
  })

  it('scale each font size from the base size', () => {
    let tokens = setAt(defaultTokens(), 'fonts.baseSize', 20)
    tokens = setAt(tokens, 'fonts.title.sizePct', 200)
    const vars = previewVars(tokens)

    expect(vars['--fk-font-base-size']).toBe('20px')
    expect(vars['--fk-font-title-size']).toBe('40px')
    expect(vars['--fk-font-title-weight']).toBe('600')
    expect(vars['--fk-font-body-size']).toBe('17.5px')
  })

  it('carry the radii, the border weights and the spacing in pixels', () => {
    let tokens = setAt(defaultTokens(), 'borders.widgetRadius', 12)
    tokens = setAt(tokens, 'borders.socialButtonBorderWeight', 3)
    tokens = setAt(tokens, 'spacing.fieldGap', 40)
    const vars = previewVars(tokens)

    expect(vars['--fk-radius-widget']).toBe('12px')
    expect(vars['--fk-border-social-button']).toBe('3px')
    expect(vars['--fk-spacing-field-gap']).toBe('40px')
    expect(vars['--fk-spacing-widget-padding']).toBe('24px')
  })

  it('turn the shadow and the link style into css', () => {
    expect(
      previewVars(setAt(defaultTokens(), 'borders.widgetShadow', 'none'))['--fk-shadow-widget']
    ).toBe('none')
    expect(previewVars(defaultTokens())['--fk-shadow-widget']).toContain('rgba')
    expect(
      previewVars(setAt(defaultTokens(), 'fonts.links.style', 'underline'))[
        '--fk-font-link-decoration'
      ]
    ).toBe('underline')
    expect(previewVars(defaultTokens())['--fk-font-link-decoration']).toBe('none')
  })

  it('fall back to the default for what is half typed', () => {
    let tokens = setAt(defaultTokens(), 'colors.primaryButton', '#ff00')
    tokens = setAt(tokens, 'borders.widgetRadius', Number.NaN)
    const vars = previewVars(tokens)

    expect(vars['--fk-color-primary-button']).toBe('#635dff')
    expect(vars['--fk-radius-widget']).toBe('5px')
  })

  it('lowercase and trim a colour', () => {
    expect(
      previewVars(setAt(defaultTokens(), 'colors.error', ' #ABCDEF '))['--fk-color-error']
    ).toBe('#abcdef')
  })
})
