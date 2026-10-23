import type { Schemas } from '@/api/api.client'
import {
  BORDER_KEYS,
  COLOR_KEYS,
  FONT_STYLE_KEYS,
  MAX_NAME_LENGTH,
  SPACING_KEYS,
  firstProblem,
  toTokens,
  withDefaults,
} from './theme-tokens'

export const EXPORT_MARKER = 'autharie_theme'
export const EXPORT_VERSION = 1
export const MAX_THEME_BYTES = 32 * 1024
export const MAX_FILE_BYTES = 1024 * 1024

const FALLBACK_NAME = 'Imported theme'

export interface ThemeFile {
  name: string
  tokens: Schemas.ThemeTokens
}

export type ImportResult = { ok: true; theme: ThemeFile } | { ok: false; error: string }

class Refusal extends Error {}

function bytes(text: string): number {
  return new TextEncoder().encode(text).length
}

function plain(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

export function exportTheme(theme: ThemeFile): string {
  return JSON.stringify(
    {
      [EXPORT_MARKER]: EXPORT_VERSION,
      name: theme.name,
      tokens: toTokens(withDefaults(theme.tokens)),
    },
    null,
    2
  )
}

export function exportFileName(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return `${slug === '' ? 'theme' : slug}.json`
}

function pick(source: unknown, path: string, keys: readonly string[]): Record<string, unknown> {
  if (!plain(source)) throw new Refusal(`${path}: must be a JSON object.`)
  return Object.fromEntries(keys.filter((key) => key in source).map((key) => [key, source[key]]))
}

function pickStyle(source: unknown, path: string, keys: readonly string[]): unknown {
  return source === undefined || source === null ? undefined : pick(source, path, keys)
}

function pickTokens(source: unknown): Schemas.ThemeTokens {
  if (!plain(source)) throw new Refusal('tokens: must be a JSON object.')

  const picked: Record<string, unknown> = {}
  const section = (key: string, keys: readonly string[]) => {
    if (source[key] !== undefined && source[key] !== null)
      picked[key] = pick(source[key], key, keys)
  }

  section('colors', COLOR_KEYS)
  section('borders', [...BORDER_KEYS, 'widgetShadow'])
  section('spacing', SPACING_KEYS)

  if (source.fonts !== undefined && source.fonts !== null) {
    const fonts = pick(source.fonts, 'fonts', ['url', 'baseSize', ...FONT_STYLE_KEYS, 'links'])
    for (const key of FONT_STYLE_KEYS) {
      fonts[key] = pickStyle(fonts[key], `fonts.${key}`, ['weight', 'sizePct'])
    }
    fonts.links = pickStyle(fonts.links, 'fonts.links', ['weight', 'sizePct', 'style'])
    picked.fonts = Object.fromEntries(
      Object.entries(fonts).filter(([, value]) => value !== undefined)
    )
  }

  return picked as Schemas.ThemeTokens
}

function cleanName(value: unknown): string {
  if (typeof value !== 'string') return FALLBACK_NAME
  const text = Array.from(value)
    .map((char) => {
      const code = char.charCodeAt(0)
      return code < 32 || (code >= 127 && code < 160) ? ' ' : char
    })
    .join('')
    .trim()
  const name = Array.from(text).slice(0, MAX_NAME_LENGTH).join('').trim()
  return name === '' ? FALLBACK_NAME : name
}

function tokensOf(file: Record<string, unknown>): unknown {
  if (EXPORT_MARKER in file) {
    if (file[EXPORT_MARKER] !== EXPORT_VERSION) {
      throw new Refusal(
        `This theme file is version ${String(file[EXPORT_MARKER])}; this console reads version ${EXPORT_VERSION}.`
      )
    }
    if (file.tokens === undefined || file.tokens === null)
      throw new Refusal('The file has no tokens.')
    return file.tokens
  }

  if ('ferriskey' in file) {
    if (file.config === undefined || file.config === null) {
      throw new Refusal('The FerrisKey export has no config.')
    }
    return file.config
  }

  throw new Refusal('This file is neither an Autharie theme nor a FerrisKey export.')
}

function read(text: string): ThemeFile {
  if (bytes(text) > MAX_FILE_BYTES) {
    throw new Refusal(`The file is larger than ${MAX_FILE_BYTES / 1024 / 1024} MiB.`)
  }

  let file: unknown
  try {
    file = JSON.parse(text)
  } catch {
    throw new Refusal('This is not valid JSON.')
  }
  if (!plain(file)) throw new Refusal('Expected a JSON object.')

  const raw = tokensOf(file)
  const size = bytes(JSON.stringify(raw))
  if (size > MAX_THEME_BYTES) {
    throw new Refusal(`The tokens are ${size} bytes, more than the ${MAX_THEME_BYTES} allowed.`)
  }

  const complete = withDefaults(pickTokens(raw))
  const problem = firstProblem(complete)
  if (problem) throw new Refusal(problem)

  return { name: cleanName(file.name), tokens: toTokens(complete) }
}

export function parseImport(text: string): ImportResult {
  try {
    return { ok: true, theme: read(text) }
  } catch (error) {
    if (error instanceof Refusal) return { ok: false, error: error.message }
    throw error
  }
}
