import type { SealingJwk } from './types'

export interface SealingKeyPair {
  privateKey: CryptoKey
  publicJwk: SealingJwk
}

const MODULUS_BITS = 2048
const PUBLIC_EXPONENT = new Uint8Array([1, 0, 1])

function toBytes(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.replace(/-/g, '+').replace(/_/g, '/')
  const padded = base64 + '='.repeat((4 - (base64.length % 4)) % 4)
  const raw = atob(padded)
  const bytes = new Uint8Array(raw.length)
  for (let index = 0; index < raw.length; index += 1) bytes[index] = raw.charCodeAt(index)
  return bytes
}

export function toBase64Url(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)
  let raw = ''
  for (const byte of view) raw += String.fromCharCode(byte)
  return btoa(raw).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

export async function generateSealingKeyPair(): Promise<SealingKeyPair> {
  const pair = await crypto.subtle.generateKey(
    {
      name: 'RSA-OAEP',
      modulusLength: MODULUS_BITS,
      publicExponent: PUBLIC_EXPONENT,
      hash: 'SHA-256',
    },
    false,
    ['encrypt', 'decrypt'],
  )
  const jwk = await crypto.subtle.exportKey('jwk', pair.publicKey)
  if (!jwk.n || !jwk.e) throw new Error('The browser did not export a public key.')

  return {
    privateKey: pair.privateKey,
    publicJwk: { kty: 'RSA', n: jwk.n, e: jwk.e, alg: 'RSA-OAEP-256' },
  }
}

export async function openSealed(privateKey: CryptoKey, sealed: string): Promise<string> {
  const clear = await crypto.subtle.decrypt({ name: 'RSA-OAEP' }, privateKey, toBytes(sealed))
  return new TextDecoder().decode(clear)
}
