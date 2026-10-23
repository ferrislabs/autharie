import { describe, expect, it } from 'vitest'
import { generateSealingKeyPair, openSealed, toBase64Url } from './credential-crypto'

async function seal(jwk: { n: string; e: string }, secret: string): Promise<string> {
  const publicKey = await crypto.subtle.importKey(
    'jwk',
    { kty: 'RSA', n: jwk.n, e: jwk.e, alg: 'RSA-OAEP-256', ext: true },
    { name: 'RSA-OAEP', hash: 'SHA-256' },
    false,
    ['encrypt'],
  )
  const sealed = await crypto.subtle.encrypt(
    { name: 'RSA-OAEP' },
    publicKey,
    new TextEncoder().encode(secret),
  )
  return toBase64Url(sealed)
}

describe('the sealing key pair', () => {
  it('is a 2048 bit RSA-OAEP-256 key whose private half cannot be exported', async () => {
    const pair = await generateSealingKeyPair()

    expect(pair.publicJwk.kty).toBe('RSA')
    expect(pair.publicJwk.alg).toBe('RSA-OAEP-256')
    expect(pair.publicJwk.e).toBe('AQAB')
    expect(Object.keys(pair.publicJwk).sort()).toEqual(['alg', 'e', 'kty', 'n'])
    expect(atob(pair.publicJwk.n.replace(/-/g, '+').replace(/_/g, '/')).length).toBe(256)
    expect(pair.privateKey.extractable).toBe(false)
    await expect(crypto.subtle.exportKey('jwk', pair.privateKey)).rejects.toThrow()
  })

  it('opens what was sealed to its public half, as the data plane seals it', async () => {
    const pair = await generateSealingKeyPair()
    const sealed = await seal(pair.publicJwk, 's3cret-ü-value')

    expect(await openSealed(pair.privateKey, sealed)).toBe('s3cret-ü-value')
    expect(sealed).toMatch(/^[A-Za-z0-9_-]+$/)
    expect(sealed).not.toContain('s3cret')
  })

  it('does not open what was sealed to another key', async () => {
    const mine = await generateSealingKeyPair()
    const other = await generateSealingKeyPair()
    const sealed = await seal(other.publicJwk, 'not for me')

    await expect(openSealed(mine.privateKey, sealed)).rejects.toThrow()
  })

  it('makes a new pair every time', async () => {
    const [a, b] = await Promise.all([generateSealingKeyPair(), generateSealingKeyPair()])

    expect(a.publicJwk.n).not.toBe(b.publicJwk.n)
  })
})
