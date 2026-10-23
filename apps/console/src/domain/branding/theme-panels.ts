import {
  TOKEN_PATHS,
  defaultTokens,
  getAt,
  setAt,
  type TokenProblems,
  type Tokens,
} from './theme-tokens'

export type PanelId = 'colors' | 'typography' | 'buttons' | 'inputs' | 'widget' | 'page'

export interface Panel {
  id: PanelId
  label: string
  description: string
}

export const PANELS: readonly Panel[] = [
  { id: 'colors', label: 'Colors', description: 'The main palette of the login page.' },
  { id: 'typography', label: 'Typography', description: 'Font, sizes and weights of the text.' },
  {
    id: 'buttons',
    label: 'Buttons',
    description: 'Secondary, social, passkey and magic link buttons.',
  },
  { id: 'inputs', label: 'Inputs', description: 'The fields people type into.' },
  { id: 'widget', label: 'Widget', description: 'The card holding the form.' },
  { id: 'page', label: 'Page spacing', description: 'The gaps between fields and sections.' },
]

function under(...prefixes: string[]): string[] {
  return TOKEN_PATHS.filter((path) =>
    prefixes.some((prefix) => path === prefix || path.startsWith(`${prefix}.`))
  )
}

function family(...prefixes: string[]): string[] {
  return TOKEN_PATHS.filter((path) => prefixes.some((prefix) => path.startsWith(prefix)))
}

export const PANEL_PATHS: Record<PanelId, readonly string[]> = {
  colors: [
    'colors.primaryButton',
    'colors.primaryButtonLabel',
    'colors.links',
    'colors.pageBackground',
    'colors.widgetBackground',
    'colors.bodyText',
    'colors.error',
  ],
  typography: under(
    'fonts.url',
    'fonts.baseSize',
    'fonts.title',
    'fonts.subtitle',
    'fonts.body',
    'fonts.links'
  ),
  buttons: [
    ...family(
      'colors.secondaryButton',
      'colors.socialButton',
      'colors.magicLinkButton',
      'colors.passkeyButton'
    ),
    ...family(
      'borders.buttonRadius',
      'borders.buttonBorderWeight',
      'borders.socialButton',
      'borders.magicLinkButton',
      'borders.passkeyButton'
    ),
    ...under('fonts.buttons'),
  ],
  inputs: [...family('borders.input'), ...under('fonts.inputLabels')],
  widget: [...family('borders.widget'), 'spacing.widgetPadding'],
  page: ['spacing.fieldGap', 'spacing.sectionGap'],
}

const HOME = new Map<string, PanelId>(
  PANELS.flatMap(({ id }) => PANEL_PATHS[id].map((path) => [path, id] as const))
)

export function panelOf(path: string): PanelId {
  const home = HOME.get(path)
  if (!home) throw new Error(`no panel for ${path}`)
  return home
}

export function panelsWithProblems(problems: TokenProblems): ReadonlySet<PanelId> {
  return new Set(Object.keys(problems).map(panelOf))
}

export function resetPanel(tokens: Tokens, id: PanelId): Tokens {
  const fallback = defaultTokens()

  return TOKEN_PATHS.filter((path) => panelOf(path) === id).reduce(
    (current, path) => setAt(current, path, getAt(fallback, path)),
    tokens
  )
}
