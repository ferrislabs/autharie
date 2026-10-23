import type { Schemas } from '@/api/api.client'

export const COLOR_KEYS = [
  'primaryButton',
  'primaryButtonLabel',
  'secondaryButton',
  'secondaryButtonLabel',
  'socialButtonBackground',
  'socialButtonLabel',
  'socialButtonBorder',
  'magicLinkButtonBackground',
  'magicLinkButtonLabel',
  'magicLinkButtonBorder',
  'passkeyButtonBackground',
  'passkeyButtonLabel',
  'passkeyButtonBorder',
  'widgetBackground',
  'pageBackground',
  'bodyText',
  'links',
  'error',
] as const

export const BORDER_KEYS = [
  'buttonRadius',
  'buttonBorderWeight',
  'socialButtonBorderWeight',
  'magicLinkButtonBorderWeight',
  'passkeyButtonBorderWeight',
  'inputRadius',
  'inputBorderWeight',
  'widgetRadius',
  'widgetBorderWeight',
] as const

export const SPACING_KEYS = ['widgetPadding', 'fieldGap', 'sectionGap'] as const

export const FONT_STYLE_KEYS = ['title', 'subtitle', 'body', 'buttons', 'inputLabels'] as const

export const SHADOWS = ['none', 'small', 'large'] as const
export const LINK_STYLES = ['normal', 'underline'] as const

export type ColorKey = (typeof COLOR_KEYS)[number]
export type BorderKey = (typeof BORDER_KEYS)[number]
export type SpacingKey = (typeof SPACING_KEYS)[number]
export type FontStyleKey = (typeof FONT_STYLE_KEYS)[number]
export type Shadow = Schemas.ThemeShadow
export type LinkStyle = Schemas.ThemeLinkStyle

export interface FontStyle {
  weight: number
  sizePct: number
}

export interface LinkFont extends FontStyle {
  style: LinkStyle
}

export interface Tokens {
  colors: Record<ColorKey, string>
  fonts: Record<FontStyleKey, FontStyle> & {
    url: string | null
    baseSize: number
    links: LinkFont
  }
  borders: Record<BorderKey, number> & { widgetShadow: Shadow }
  spacing: Record<SpacingKey, number>
}

export interface ThemeDraft {
  name: string
  tokens: Tokens
}

export type TokenProblems = Record<string, string>

export const MAX_NAME_LENGTH = 80
export const MAX_FONT_URL_LENGTH = 2048
export const LEGACY_THEME_NAME = 'Current branding'

export const RANGES = {
  baseSize: { min: 10, max: 24, step: 1 },
  weight: { min: 100, max: 900, step: 100 },
  sizePct: { min: 50, max: 300, step: 2.5 },
  border: { min: 0, max: 64, step: 1 },
  spacing: { min: 0, max: 128, step: 1 },
} as const

const DEFAULTS: Tokens = {
  colors: {
    primaryButton: '#635dff',
    primaryButtonLabel: '#ffffff',
    secondaryButton: '#ffffff',
    secondaryButtonLabel: '#1e212a',
    socialButtonBackground: '#ffffff',
    socialButtonLabel: '#1e212a',
    socialButtonBorder: '#d1d5db',
    magicLinkButtonBackground: '#ffffff',
    magicLinkButtonLabel: '#1e212a',
    magicLinkButtonBorder: '#d1d5db',
    passkeyButtonBackground: '#ffffff',
    passkeyButtonLabel: '#1e212a',
    passkeyButtonBorder: '#d1d5db',
    widgetBackground: '#ffffff',
    pageBackground: '#000000',
    bodyText: '#1e212a',
    links: '#635dff',
    error: '#d03c38',
  },
  fonts: {
    url: null,
    baseSize: 16,
    title: { weight: 600, sizePct: 150 },
    subtitle: { weight: 400, sizePct: 87.5 },
    body: { weight: 400, sizePct: 87.5 },
    buttons: { weight: 600, sizePct: 100 },
    inputLabels: { weight: 500, sizePct: 100 },
    links: { weight: 600, sizePct: 87.5, style: 'normal' },
  },
  borders: {
    buttonRadius: 3,
    buttonBorderWeight: 1,
    socialButtonBorderWeight: 1,
    magicLinkButtonBorderWeight: 1,
    passkeyButtonBorderWeight: 1,
    inputRadius: 3,
    inputBorderWeight: 1,
    widgetRadius: 5,
    widgetBorderWeight: 1,
    widgetShadow: 'large',
  },
  spacing: {
    widgetPadding: 24,
    fieldGap: 16,
    sectionGap: 24,
  },
}

export const TOKEN_PATHS: readonly string[] = [
  ...COLOR_KEYS.map((key) => `colors.${key}`),
  'fonts.url',
  'fonts.baseSize',
  ...FONT_STYLE_KEYS.flatMap((key) => [`fonts.${key}.weight`, `fonts.${key}.sizePct`]),
  'fonts.links.weight',
  'fonts.links.sizePct',
  'fonts.links.style',
  ...BORDER_KEYS.map((key) => `borders.${key}`),
  'borders.widgetShadow',
  ...SPACING_KEYS.map((key) => `spacing.${key}`),
]

export function defaultTokens(): Tokens {
  return structuredClone(DEFAULTS)
}

export function getAt(tokens: Tokens, path: string): unknown {
  return path
    .split('.')
    .reduce<unknown>(
      (node, key) =>
        node !== null && typeof node === 'object'
          ? (node as Record<string, unknown>)[key]
          : undefined,
      tokens
    )
}

export function setAt(tokens: Tokens, path: string, value: unknown): Tokens {
  const next = structuredClone(tokens)
  const keys = path.split('.')
  const last = keys[keys.length - 1]
  let node = next as unknown as Record<string, unknown>
  for (const key of keys.slice(0, -1)) node = node[key] as Record<string, unknown>
  node[last] = value
  return next
}

function style<T extends FontStyle>(given: Partial<T> | null | undefined, fallback: T): T {
  return { ...fallback, ...stripNull(given) }
}

function stripNull<T extends object>(value: T | null | undefined): Partial<T> {
  if (!value) return {}
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== null && entry !== undefined)
  ) as Partial<T>
}

export function withDefaults(tokens: Schemas.ThemeTokens | null | undefined): Tokens {
  const base = defaultTokens()
  const fonts = tokens?.fonts

  return {
    colors: { ...base.colors, ...stripNull(tokens?.colors) } as Tokens['colors'],
    fonts: {
      url: fonts?.url ?? null,
      baseSize: fonts?.baseSize ?? base.fonts.baseSize,
      title: style(fonts?.title, base.fonts.title),
      subtitle: style(fonts?.subtitle, base.fonts.subtitle),
      body: style(fonts?.body, base.fonts.body),
      buttons: style(fonts?.buttons, base.fonts.buttons),
      inputLabels: style(fonts?.inputLabels, base.fonts.inputLabels),
      links: style(fonts?.links, base.fonts.links),
    },
    borders: { ...base.borders, ...stripNull(tokens?.borders) } as Tokens['borders'],
    spacing: { ...base.spacing, ...stripNull(tokens?.spacing) } as Tokens['spacing'],
  }
}

function hex(value: unknown): string {
  return typeof value === 'string' ? value.trim().toLowerCase() : ''
}

export function toTokens(tokens: Tokens): Schemas.ThemeTokens {
  const colors = Object.fromEntries(COLOR_KEYS.map((key) => [key, hex(tokens.colors[key])]))
  const url = tokens.fonts.url?.trim()

  return {
    colors,
    fonts: {
      ...(url ? { url } : {}),
      baseSize: tokens.fonts.baseSize,
      title: { ...tokens.fonts.title },
      subtitle: { ...tokens.fonts.subtitle },
      body: { ...tokens.fonts.body },
      buttons: { ...tokens.fonts.buttons },
      inputLabels: { ...tokens.fonts.inputLabels },
      links: { ...tokens.fonts.links },
    },
    borders: { ...tokens.borders },
    spacing: { ...tokens.spacing },
  }
}

const HEX = /^#[0-9a-fA-F]{6}$/

export function isHex(value: unknown): boolean {
  return typeof value === 'string' && HEX.test(value.trim())
}

function isWhole(value: unknown, min: number, max: number): boolean {
  return typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max
}

const COLOUR_PROBLEM = 'Must be a #rrggbb colour, for example #635dff.'
const URL_PROBLEM = 'Must be an https URL.'

function wholeProblem(value: unknown, min: number, max: number): string | null {
  return isWhole(value, min, max) ? null : `Must be a whole number between ${min} and ${max}.`
}

function weightProblem(value: unknown): string | null {
  const { min, max, step } = RANGES.weight
  if (!isWhole(value, min, max)) return `Must be a whole number between ${min} and ${max}.`
  return (value as number) % step === 0
    ? null
    : 'Must be a weight in steps of 100: 100, 200, ... 900.'
}

function sizePctProblem(value: unknown): string | null {
  const { min, max } = RANGES.sizePct
  return typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max
    ? null
    : `Must be a number between ${min} and ${max}.`
}

function choiceProblem(value: unknown, allowed: readonly string[]): string | null {
  return typeof value === 'string' && allowed.includes(value)
    ? null
    : `Must be one of ${allowed.join(', ')}.`
}

function urlProblem(value: unknown): string | null {
  if (value === null || value === undefined) return null
  if (typeof value !== 'string') return URL_PROBLEM
  const url = value.trim()
  if (url === '') return null
  if (url.length > MAX_FONT_URL_LENGTH) return `Must be at most ${MAX_FONT_URL_LENGTH} characters.`
  const authority = url.startsWith('https://') ? url.slice('https://'.length).split(/[/?#]/)[0] : ''
  const unclean = Array.from(url).some(
    (char) => /\s/.test(char) || char.charCodeAt(0) < 32 || '"\'<>\\`()'.includes(char)
  )
  return authority === '' || authority.includes('@') || unclean ? URL_PROBLEM : null
}

export function validateTokens(tokens: Tokens): TokenProblems {
  const problems: TokenProblems = {}
  const report = (path: string, problem: string | null) => {
    if (problem) problems[path] = problem
  }

  for (const key of COLOR_KEYS) {
    report(`colors.${key}`, isHex(tokens.colors[key]) ? null : COLOUR_PROBLEM)
  }

  report('fonts.url', urlProblem(tokens.fonts.url))
  report(
    'fonts.baseSize',
    wholeProblem(tokens.fonts.baseSize, RANGES.baseSize.min, RANGES.baseSize.max)
  )
  for (const key of [...FONT_STYLE_KEYS, 'links'] as const) {
    report(`fonts.${key}.weight`, weightProblem(tokens.fonts[key].weight))
    report(`fonts.${key}.sizePct`, sizePctProblem(tokens.fonts[key].sizePct))
  }
  report('fonts.links.style', choiceProblem(tokens.fonts.links.style, LINK_STYLES))

  for (const key of BORDER_KEYS) {
    report(
      `borders.${key}`,
      wholeProblem(tokens.borders[key], RANGES.border.min, RANGES.border.max)
    )
  }
  report('borders.widgetShadow', choiceProblem(tokens.borders.widgetShadow, SHADOWS))

  for (const key of SPACING_KEYS) {
    report(
      `spacing.${key}`,
      wholeProblem(tokens.spacing[key], RANGES.spacing.min, RANGES.spacing.max)
    )
  }

  return problems
}

export function firstProblem(tokens: Tokens): string | null {
  const [first] = Object.entries(validateTokens(tokens))
  return first ? `${first[0]}: ${first[1]}` : null
}

export function effectiveTokens(tokens: Tokens): Tokens {
  const fallback = defaultTokens()

  return Object.keys(validateTokens(tokens)).reduce(
    (current, path) => setAt(current, path, getAt(fallback, path)),
    tokens
  )
}

export function describeNameProblem(name: string): string | null {
  const trimmed = name.trim()
  const length = Array.from(trimmed).length
  if (length === 0 || length > MAX_NAME_LENGTH) {
    return `Use between 1 and ${MAX_NAME_LENGTH} characters.`
  }
  const control = Array.from(trimmed).some((char) => {
    const code = char.charCodeAt(0)
    return code < 32 || (code >= 127 && code < 160)
  })
  return control ? 'May not contain control characters.' : null
}

export function isValid(draft: ThemeDraft): boolean {
  return (
    describeNameProblem(draft.name) === null &&
    Object.keys(validateTokens(draft.tokens)).length === 0
  )
}

export function isDirty(
  draft: ThemeDraft,
  stored: { name: string; tokens: Schemas.ThemeTokens }
): boolean {
  return (
    draft.name.trim() !== stored.name ||
    JSON.stringify(toTokens(draft.tokens)) !== JSON.stringify(toTokens(withDefaults(stored.tokens)))
  )
}

export function legacyToTokens(branding: Schemas.Branding | null | undefined): Schemas.ThemeTokens {
  const tokens: Schemas.ThemeTokens = {}
  const colors = branding?.colors
  const pairs: Array<[keyof Schemas.ThemeColors, string | null | undefined]> = [
    ['primaryButton', colors?.primary],
    ['primaryButtonLabel', colors?.primary_text],
    ['links', colors?.links],
    ['pageBackground', colors?.page_background],
    ['widgetBackground', colors?.widget_background],
    ['bodyText', colors?.text],
    ['error', colors?.error],
  ]
  const mapped = Object.fromEntries(pairs.filter(([, value]) => value != null))
  if (Object.keys(mapped).length > 0) tokens.colors = mapped

  const radius = branding?.radius
  if (radius != null) {
    tokens.borders = { widgetRadius: radius, buttonRadius: radius, inputRadius: radius }
  }

  return tokens
}

export function hasLegacyBranding(branding: Schemas.Branding | null | undefined): boolean {
  const tokens = legacyToTokens(branding)
  return tokens.colors !== undefined || tokens.borders !== undefined
}
