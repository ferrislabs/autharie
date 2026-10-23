import { describe, expect, it } from 'vitest'
import { PANELS, PANEL_PATHS, panelOf, panelsWithProblems, resetPanel } from './theme-panels'
import { TOKEN_PATHS, defaultTokens, getAt, setAt } from './theme-tokens'

describe('the builder panels', () => {
  it('are Colors, Typography, Buttons, Inputs, Widget and Page spacing, in that order', () => {
    expect(PANELS.map((panel) => panel.label)).toEqual([
      'Colors',
      'Typography',
      'Buttons',
      'Inputs',
      'Widget',
      'Page spacing',
    ])
  })

  it('give every token exactly one home', () => {
    for (const path of TOKEN_PATHS) {
      const homes = PANELS.filter((panel) => panelOf(path) === panel.id)
      expect(homes, path).toHaveLength(1)
    }
  })

  it('never claim a token twice, and miss none', () => {
    const claimed = PANELS.flatMap(({ id }) => PANEL_PATHS[id])

    expect(new Set(claimed).size).toBe(claimed.length)
    expect([...claimed].sort()).toEqual([...TOKEN_PATHS].sort())
  })

  it('know where a few tokens live', () => {
    expect(panelOf('colors.primaryButton')).toBe('colors')
    expect(panelOf('colors.socialButtonBorder')).toBe('buttons')
    expect(panelOf('fonts.buttons.weight')).toBe('buttons')
    expect(panelOf('fonts.inputLabels.sizePct')).toBe('inputs')
    expect(panelOf('fonts.links.style')).toBe('typography')
    expect(panelOf('fonts.url')).toBe('typography')
    expect(panelOf('borders.widgetShadow')).toBe('widget')
    expect(panelOf('spacing.widgetPadding')).toBe('widget')
    expect(panelOf('spacing.fieldGap')).toBe('page')
  })

  it('flag the panels that hold a problem', () => {
    const flagged = panelsWithProblems({
      'colors.error': 'bad',
      'borders.inputRadius': 'bad',
    })

    expect([...flagged].sort()).toEqual(['colors', 'inputs'])
    expect(panelsWithProblems({}).size).toBe(0)
  })

  it('reset only their own tokens', () => {
    let tokens = setAt(defaultTokens(), 'borders.widgetRadius', 30)
    tokens = setAt(tokens, 'borders.inputRadius', 20)
    tokens = setAt(tokens, 'colors.error', '#000000')

    const reset = resetPanel(tokens, 'widget')

    expect(reset.borders.widgetRadius).toBe(5)
    expect(reset.borders.inputRadius).toBe(20)
    expect(reset.colors.error).toBe('#000000')
  })

  it('reset a whole panel to the defaults', () => {
    for (const { id } of PANELS) {
      let tokens = defaultTokens()
      for (const path of TOKEN_PATHS) {
        if (panelOf(path) === id) tokens = setAt(tokens, path, 'broken')
      }

      const reset = resetPanel(tokens, id)

      for (const path of TOKEN_PATHS) {
        if (panelOf(path) === id)
          expect(getAt(reset, path), path).toEqual(getAt(defaultTokens(), path))
      }
    }
  })
})
