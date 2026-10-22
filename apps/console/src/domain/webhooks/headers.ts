import type { Schemas } from '@/api/api.client'

export interface HeaderRow {
  key: string
  name: string
  value: string
  stored: boolean
  replacing: boolean
}

export interface HeaderChanges {
  unchanged: string[]
  replaced: string[]
  added: string[]
  removed: string[]
}

const TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/
const RESERVED = 'x-ferriskey-signature'

export function blankHeader(key: string): HeaderRow {
  return { key, name: '', value: '', stored: false, replacing: false }
}

export function headersFromNames(names: string[]): HeaderRow[] {
  return names.map((name) => ({
    key: `stored:${name}`,
    name,
    value: '',
    stored: true,
    replacing: false,
  }))
}

function isBlank(row: HeaderRow): boolean {
  return row.name.trim() === '' && row.value === ''
}

export function toHeaderRequests(rows: HeaderRow[]): Schemas.WebhookHeaderRequest[] {
  return rows
    .filter((row) => !isBlank(row))
    .map((row) =>
      row.stored && !row.replacing
        ? { name: row.name }
        : { name: row.name.trim(), value: row.value }
    )
}

export function headerChanges(stored: string[], rows: HeaderRow[]): HeaderChanges {
  const kept = rows.filter((row) => !isBlank(row))
  const named = (pick: (row: HeaderRow) => boolean) =>
    kept.filter(pick).map((row) => row.name.trim())

  return {
    unchanged: named((row) => row.stored && !row.replacing),
    replaced: named((row) => row.stored && row.replacing),
    added: named((row) => !row.stored),
    removed: stored.filter((name) => !kept.some((row) => row.stored && row.name === name)),
  }
}

export function checkHeaders(rows: HeaderRow[]): string | null {
  const seen = new Set<string>()

  for (const row of rows.filter((row) => !isBlank(row))) {
    const name = row.name.trim()
    if (name === '') return 'Give every header a name, or remove the row.'
    if (!TOKEN.test(name)) {
      return `"${name}" is not a valid header name. Use letters, digits and dashes, without spaces.`
    }
    if (name.toLowerCase() === RESERVED) {
      return 'The signature header is set by FerrisKey and cannot be replaced.'
    }
    if (seen.has(name.toLowerCase())) return `The header "${name}" appears twice.`
    seen.add(name.toLowerCase())

    const needsValue = !row.stored || row.replacing
    if (needsValue && row.value === '') return `Give the header "${name}" a value.`
  }

  return null
}
