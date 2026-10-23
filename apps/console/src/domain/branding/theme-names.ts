import { MAX_NAME_LENGTH } from './theme-tokens'

function key(name: string): string {
  return name.trim().toLowerCase()
}

export function copyName(name: string, taken: readonly string[]): string {
  const used = new Set(taken.map(key))
  const base = name.trim()

  for (let attempt = 1; ; attempt += 1) {
    const suffix = attempt === 1 ? ' (copy)' : ` (copy ${attempt})`
    const head = Array.from(base)
      .slice(0, MAX_NAME_LENGTH - suffix.length)
      .join('')
    const candidate = `${head}${suffix}`
    if (!used.has(key(candidate))) return candidate
  }
}

export function availableName(name: string, taken: readonly string[]): string {
  const base = name.trim()
  return taken.some((entry) => key(entry) === key(base)) ? copyName(base, taken) : base
}
