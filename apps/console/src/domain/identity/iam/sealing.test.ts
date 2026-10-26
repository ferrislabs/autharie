import { afterEach, describe, expect, it, vi } from 'vitest'
import { toBase64Url } from '@/domain/agents/credential-crypto'
import fixture from './fixtures/iam-envelope.json'
import { generateSealingKeyPair, openEnvelope } from './sealing'

const REQUEST_A = '6f1c2f0e-3b1c-4d8e-9a55-0d6a7f3c9b01'
const REQUEST_B = '6f1c2f0e-3b1c-4d8e-9a55-0d6a7f3c9b02'

async function seal(
  jwk: { n: string; e: string },
  requestId: string,
  plaintext: string,
): Promise<Record<string, unknown>> {
  const publicKey = await crypto.subtle.importKey(
    'jwk',
    { kty: 'RSA', n: jwk.n, e: jwk.e, alg: 'RSA-OAEP-256', ext: true },
    { name: 'RSA-OAEP', hash: 'SHA-256' },
    false,
    ['encrypt'],
  )
  const raw = crypto.getRandomValues(new Uint8Array(32))
  const iv = crypto.getRandomValues(new Uint8Array(12))
  const aesKey = await crypto.subtle.importKey('raw', raw, { name: 'AES-GCM' }, false, ['encrypt'])
  const ct = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv, additionalData: new TextEncoder().encode(requestId) },
    aesKey,
    new TextEncoder().encode(plaintext),
  )
  const ek = await crypto.subtle.encrypt({ name: 'RSA-OAEP' }, publicKey, raw)

  return {
    v: 1,
    alg: 'RSA-OAEP-256+A256GCM',
    ek: toBase64Url(ek),
    iv: toBase64Url(iv),
    ct: toBase64Url(ct),
  }
}

async function fixtureKey(): Promise<CryptoKey> {
  const { kty, n, e, d, p, q, dp, dq, qi } = fixture.private_jwk
  const jwk = { kty, n, e, d, p, q, dp, dq, qi }

  return crypto.subtle.importKey('jwk', jwk, { name: 'RSA-OAEP', hash: 'SHA-256' }, false, [
    'decrypt',
  ])
}

function flipFirst(value: string): string {
  return (value[0] === 'A' ? 'B' : 'A') + value.slice(1)
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('the envelope Rust produces', () => {
  it('opens into exactly the plaintext it was sealed from', async () => {
    const opened = await openEnvelope(await fixtureKey(), fixture.request_id, fixture.envelope)

    expect(JSON.parse(opened)).toEqual(fixture.plaintext)
  })

  it('does not open as another request', async () => {
    const key = await fixtureKey()
    const other = flipFirst(fixture.request_id)

    await expect(openEnvelope(key, other, fixture.envelope)).rejects.toThrow()
    await expect(openEnvelope(key, REQUEST_A, fixture.envelope)).rejects.toThrow()
  })

  it('does not open once the ciphertext is touched', async () => {
    const key = await fixtureKey()
    const tampered = { ...fixture.envelope, ct: flipFirst(fixture.envelope.ct) }

    await expect(openEnvelope(key, fixture.request_id, tampered)).rejects.toThrow()
  })

  it('does not open once the tag, at the end of the ciphertext, is cut short', async () => {
    const key = await fixtureKey()
    const cut = { ...fixture.envelope, ct: fixture.envelope.ct.slice(0, -4) }

    await expect(openEnvelope(key, fixture.request_id, cut)).rejects.toThrow()
  })
})

describe('an envelope belongs to its request', () => {
  it('opens a browser-sealed envelope for its own request id', async () => {
    const pair = await generateSealingKeyPair()
    const envelope = await seal(pair.publicJwk, REQUEST_A, '{"items":[],"é":"ü"}')

    expect(await openEnvelope(pair.privateKey, REQUEST_A, envelope)).toBe('{"items":[],"é":"ü"}')
  })

  it('fails to open an envelope sealed for request A as request B', async () => {
    const pair = await generateSealingKeyPair()
    const envelope = await seal(pair.publicJwk, REQUEST_A, '{"items":[]}')

    await expect(openEnvelope(pair.privateKey, REQUEST_B, envelope)).rejects.toThrow()
  })

  it('fails to open an envelope sealed to another key', async () => {
    const mine = await generateSealingKeyPair()
    const other = await generateSealingKeyPair()
    const envelope = await seal(other.publicJwk, REQUEST_A, '{"items":[]}')

    await expect(openEnvelope(mine.privateKey, REQUEST_A, envelope)).rejects.toThrow()
  })

  it('says nothing of the content when it fails', async () => {
    const pair = await generateSealingKeyPair()
    const envelope = await seal(pair.publicJwk, REQUEST_A, 'topsecretcontent')

    await expect(openEnvelope(pair.privateKey, REQUEST_B, envelope)).rejects.toThrow(
      /could not be opened/,
    )
  })
})

describe('an envelope that is not one', () => {
  const good = fixture.envelope

  it.each([
    ['a wrong version', { ...good, v: 2 }],
    ['a wrong algorithm', { ...good, alg: 'RSA-OAEP+A128GCM' }],
    ['a missing ek', { v: good.v, alg: good.alg, iv: good.iv, ct: good.ct }],
    ['a missing iv', { v: good.v, alg: good.alg, ek: good.ek, ct: good.ct }],
    ['a missing ct', { v: good.v, alg: good.alg, ek: good.ek, iv: good.iv }],
    ['an ek that is not base64url', { ...good, ek: 'not base64url!' }],
    ['a padded ct', { ...good, ct: `${good.ct}==` }],
    ['an iv of the wrong length', { ...good, iv: toBase64Url(new Uint8Array(16)) }],
    ['a ct shorter than its tag', { ...good, ct: toBase64Url(new Uint8Array(8)) }],
    ['a number', 7],
    ['a string', 'envelope'],
    ['null', null],
  ])('is refused with %s before any crypto runs', async (_name, envelope) => {
    const key = await fixtureKey()
    const decrypt = vi.spyOn(crypto.subtle, 'decrypt')
    const importKey = vi.spyOn(crypto.subtle, 'importKey')

    await expect(openEnvelope(key, fixture.request_id, envelope)).rejects.toThrow(
      /not a sealed envelope/,
    )

    expect(decrypt).not.toHaveBeenCalled()
    expect(importKey).not.toHaveBeenCalled()
  })
})

describe('the sealing key pair', () => {
  it('is the registry one: non-extractable and able to decrypt', async () => {
    const pair = await generateSealingKeyPair()

    expect(pair.privateKey.extractable).toBe(false)
    expect(pair.privateKey.usages).toContain('decrypt')
    expect(pair.publicJwk).toMatchObject({ kty: 'RSA', alg: 'RSA-OAEP-256', e: 'AQAB' })
  })
})
