export const MAX_SERVERS = 10
export const RESERVED_SLUGS = ['ferriskey', 'autharie', 'gateway']

const SLUG = /^[a-z][a-z0-9-]{1,30}$/
const MAX_NAME = 100
const MAX_URL = 2048
const INTERNAL_SUFFIXES = ['.local', '.localhost', '.localdomain', '.internal', '.intranet', '.lan']

export function slugFromName(name: string): string {
  const dashed = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  const lettered = dashed === '' || /^[a-z]/.test(dashed) ? dashed : `s-${dashed}`
  return lettered.slice(0, 31).replace(/-+$/g, '')
}

export function validateSlug(slug: string): string | null {
  if (!SLUG.test(slug)) {
    return 'Use 2 to 31 characters: a letter first, then lowercase letters, digits or hyphens.'
  }
  if (RESERVED_SLUGS.includes(slug)) return `“${slug}” is reserved.`
  return null
}

export function validateName(name: string): string | null {
  const trimmed = name.trim()
  if (trimmed.length === 0 || [...trimmed].length > MAX_NAME) return `Use 1 to ${MAX_NAME} characters.`
  return null
}

function ipv4(text: string): number[] | null {
  const parts = text.split('.')
  if (parts.length !== 4) return null
  const octets = parts.map((part) => (/^\d{1,3}$/.test(part) ? Number(part) : NaN))
  return octets.every((octet) => octet >= 0 && octet <= 255) ? octets : null
}

function isPublicIpv4([a, b, c]: number[]): boolean {
  return !(
    a === 0 ||
    a === 10 ||
    a === 127 ||
    a >= 224 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 192 && b === 0 && (c === 0 || c === 2)) ||
    (a === 198 && (b === 18 || b === 19)) ||
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113)
  )
}

function ipv6(text: string): number[] | null {
  let groups = text
  const tail = text.lastIndexOf(':')
  const dotted = ipv4(text.slice(tail + 1))
  if (dotted) {
    groups = `${text.slice(0, tail + 1)}${((dotted[0] << 8) | dotted[1]).toString(16)}:${((dotted[2] << 8) | dotted[3]).toString(16)}`
  }
  const halves = groups.split('::')
  if (halves.length > 2) return null
  const read = (side: string) => (side === '' ? [] : side.split(':').map((part) => parseInt(part, 16)))
  const head = read(halves[0])
  const rest = halves.length === 2 ? read(halves[1]) : []
  const filled = halves.length === 2 ? [...head, ...Array<number>(8 - head.length - rest.length).fill(0), ...rest] : head
  const valid = filled.length === 8 && filled.every((group) => Number.isInteger(group) && group >= 0 && group <= 0xffff)
  return valid ? filled : null
}

function isPublicIpv6(groups: number[]): boolean {
  const [g0, g1, g2, g3, g4, g5, g6, g7] = groups
  const embedded = (high: number, low: number) => [high >> 8, high & 0xff, low >> 8, low & 0xff]
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0xffff) {
    return isPublicIpv4(embedded(g6, g7))
  }
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0) return false
  if (g0 === 0x64 && g1 === 0xff9b && g2 === 0 && g3 === 0 && g4 === 0 && g5 === 0) {
    return isPublicIpv4(embedded(g6, g7))
  }
  if (g0 === 0x2002) return isPublicIpv4(embedded(g1, g2))
  return !(
    (g0 & 0xff00) === 0xff00 ||
    (g0 & 0xffc0) === 0xfe80 ||
    (g0 & 0xffc0) === 0xfec0 ||
    (g0 & 0xfe00) === 0xfc00 ||
    (g0 === 0x2001 && g1 === 0x0db8) ||
    (g0 === 0x0100 && g1 === 0 && g2 === 0 && g3 === 0)
  )
}

export function validateServerUrl(text: string): string | null {
  const value = text.trim()
  if (value.length === 0) return 'Enter the server’s URL.'
  if (value.length > MAX_URL) return `Use at most ${MAX_URL} characters.`

  let url: URL
  try {
    url = new URL(value)
  } catch {
    return 'This is not a URL.'
  }
  if (url.protocol !== 'https:') return 'The URL must start with https://.'
  if (url.username !== '' || url.password !== '') return 'The URL must not carry a user name or password.'
  if (url.hash !== '') return 'The URL must not carry a fragment.'

  const host = url.hostname.replace(/\.$/, '').toLowerCase()
  if (host === '') return 'The URL must name a host.'

  if (host.startsWith('[')) {
    const groups = ipv6(host.slice(1, -1))
    return groups !== null && isPublicIpv6(groups)
      ? null
      : 'The URL must not point at a private, loopback or link-local address.'
  }
  const octets = ipv4(host)
  if (octets !== null) {
    return isPublicIpv4(octets) ? null : 'The URL must not point at a private, loopback or link-local address.'
  }
  if (!host.includes('.') || INTERNAL_SUFFIXES.some((suffix) => host.endsWith(suffix))) {
    return 'The URL must name a public host, not an internal one.'
  }
  return null
}
