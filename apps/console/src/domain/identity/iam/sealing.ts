import { generateSealingKeyPair } from '@/domain/agents/credential-crypto'

export { generateSealingKeyPair }

const ALGORITHM = 'RSA-OAEP-256+A256GCM'
const IV_BYTES = 12
const TAG_BYTES = 16
const AES_KEY_BYTES = 32
const BASE64URL = /^[A-Za-z0-9_-]+$/

const NOT_AN_ENVELOPE = 'The answer is not a sealed envelope this console understands.'
const NOT_OPENED = 'The answer could not be opened.'

function fromBase64Url(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.replace(/-/g, '+').replace(/_/g, '/')
  const raw = atob(base64 + '='.repeat((4 - (base64.length % 4)) % 4))
  const bytes = new Uint8Array(raw.length)
  for (let index = 0; index < raw.length; index += 1) bytes[index] = raw.charCodeAt(index)
  return bytes
}

function field(value: Record<string, unknown>, name: string): string {
  const found = value[name]
  if (typeof found !== 'string' || !BASE64URL.test(found) || found.length % 4 === 1) {
    throw new Error(NOT_AN_ENVELOPE)
  }
  return found
}

interface Parts {
  ek: Uint8Array<ArrayBuffer>
  iv: Uint8Array<ArrayBuffer>
  ct: Uint8Array<ArrayBuffer>
}

function parts(value: unknown): Parts {
  if (typeof value !== 'object' || value === null) throw new Error(NOT_AN_ENVELOPE)
  const candidate = value as Record<string, unknown>
  if (candidate.v !== 1 || candidate.alg !== ALGORITHM) throw new Error(NOT_AN_ENVELOPE)

  const ek = fromBase64Url(field(candidate, 'ek'))
  const iv = fromBase64Url(field(candidate, 'iv'))
  const ct = fromBase64Url(field(candidate, 'ct'))
  if (iv.length !== IV_BYTES || ct.length < TAG_BYTES) throw new Error(NOT_AN_ENVELOPE)

  return { ek, iv, ct }
}

export async function openEnvelope(
  privateKey: CryptoKey,
  requestId: string,
  envelope: unknown,
): Promise<string> {
  const { ek, iv, ct } = parts(envelope)

  try {
    const raw = await crypto.subtle.decrypt({ name: 'RSA-OAEP' }, privateKey, ek)
    if (raw.byteLength !== AES_KEY_BYTES) throw new Error(NOT_OPENED)

    const aesKey = await crypto.subtle.importKey('raw', raw, { name: 'AES-GCM' }, false, [
      'decrypt',
    ])
    const clear = await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv, additionalData: new TextEncoder().encode(requestId) },
      aesKey,
      ct,
    )
    return new TextDecoder('utf-8', { fatal: true }).decode(clear)
  } catch {
    throw new Error(NOT_OPENED)
  }
}
