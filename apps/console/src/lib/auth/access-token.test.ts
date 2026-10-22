import { describe, expect, it } from 'vitest'
import type { User } from 'oidc-client-ts'
import { accessTokenFor, expiryOf, singleFlight, type TokenSource } from './access-token'

const NOW = 1_700_000_000

function jwt(claims: Record<string, unknown>): string {
  const base64url = (value: unknown) =>
    btoa(JSON.stringify(value)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')

  return `${base64url({ alg: 'RS256' })}.${base64url(claims)}.signature`
}

function user(accessToken: string): User {
  return { access_token: accessToken } as User
}

class FakeSource implements TokenSource {
  renewals = 0
  adopted: User[] = []

  constructor(
    private readonly accessToken: string | null,
    private readonly session: { expiresAt?: number; refreshToken?: string } | null,
    private readonly onRenew: () => Promise<User | null> = async () => user('renewed')
  ) {}

  stored() {
    return { accessToken: this.accessToken, session: this.session }
  }

  renew() {
    this.renewals += 1

    return this.onRenew()
  }

  adopt(next: User) {
    this.adopted.push(next)
  }
}

describe('expiryOf', () => {
  it('reads the expiry a token carries', () => {
    expect(expiryOf(jwt({ exp: NOW + 300 }))).toBe(NOW + 300)
  })

  it('reads a payload whose base64url alphabet needs the padding put back', () => {
    expect(expiryOf(jwt({ exp: NOW, name: 'ça va ?>>>' }))).toBe(NOW)
  })

  it('says nothing about what is not a token', () => {
    expect(expiryOf('not-a-token')).toBeUndefined()
    expect(expiryOf('a.%%%.c')).toBeUndefined()
    expect(expiryOf(jwt({ sub: 'me' }))).toBeUndefined()
    expect(expiryOf(jwt({ exp: 'soon' }))).toBeUndefined()
  })
})

describe('the token a request leaves with', () => {
  it('is the one stored while it has time left', async () => {
    const token = jwt({ exp: NOW + 600 })
    const source = new FakeSource(token, { expiresAt: NOW + 600, refreshToken: 'r' })

    expect(await accessTokenFor(source, NOW)).toBe(token)
    expect(source.renewals).toBe(0)
  })

  it('is a renewed one when the stored one is about to expire', async () => {
    const source = new FakeSource(jwt({ exp: NOW + 30 }), { expiresAt: NOW + 30, refreshToken: 'r' })

    expect(await accessTokenFor(source, NOW)).toBe('renewed')
    expect(source.renewals).toBe(1)
  })

  it('is a renewed one when the stored one has already expired', async () => {
    const source = new FakeSource(jwt({ exp: NOW - 5 }), { expiresAt: NOW - 5, refreshToken: 'r' })

    expect(await accessTokenFor(source, NOW)).toBe('renewed')
  })

  it('goes by the expiry the token carries when the session claims a later one', async () => {
    const source = new FakeSource(jwt({ exp: NOW - 5 }), { expiresAt: NOW + 3600, refreshToken: 'r' })

    expect(await accessTokenFor(source, NOW)).toBe('renewed')
  })

  it('goes by the expiry of the session when the token does not say', async () => {
    const source = new FakeSource('opaque-token', { expiresAt: NOW - 5, refreshToken: 'r' })

    expect(await accessTokenFor(source, NOW)).toBe('renewed')
  })

  it('hands the renewed session to the store, so the next request does not renew again', async () => {
    const source = new FakeSource(jwt({ exp: NOW - 5 }), { expiresAt: NOW - 5, refreshToken: 'r' })

    await accessTokenFor(source, NOW)

    expect(source.adopted.map((adopted) => adopted.access_token)).toEqual(['renewed'])
  })

  it('leaves as it is when there is nothing to renew with', async () => {
    const token = jwt({ exp: NOW - 5 })
    const source = new FakeSource(token, { expiresAt: NOW - 5 })

    expect(await accessTokenFor(source, NOW)).toBe(token)
    expect(source.renewals).toBe(0)
  })

  it('leaves as it is when the renewal found no session', async () => {
    const token = jwt({ exp: NOW - 5 })
    const source = new FakeSource(token, { expiresAt: NOW - 5, refreshToken: 'r' }, async () => null)

    expect(await accessTokenFor(source, NOW)).toBe(token)
    expect(source.adopted).toEqual([])
  })

  it('leaves as it is when the renewal was refused, for the platform to answer', async () => {
    const token = jwt({ exp: NOW - 5 })
    const source = new FakeSource(token, { expiresAt: NOW - 5, refreshToken: 'r' }, async () => {
      throw new Error('invalid_grant')
    })

    expect(await accessTokenFor(source, NOW)).toBe(token)
    expect(source.adopted).toEqual([])
  })

  it('does not wait for a renewal that never answers', async () => {
    const token = jwt({ exp: NOW - 5 })
    const source = new FakeSource(
      token,
      { expiresAt: NOW - 5, refreshToken: 'r' },
      () => new Promise<User | null>(() => {})
    )

    expect(await accessTokenFor(source, NOW, 20)).toBe(token)
  })

  it('has nothing to send when nobody is signed in', async () => {
    const source = new FakeSource(null, null)

    expect(await accessTokenFor(source, NOW)).toBeNull()
    expect(source.renewals).toBe(0)
  })
})

describe('singleFlight', () => {
  it('shares one run between the callers that arrive while it is running', async () => {
    let runs = 0
    let release: () => void = () => {}
    const run = singleFlight(async () => {
      runs += 1
      await new Promise<void>((resolve) => (release = resolve))

      return runs
    })

    const calls = [run(), run(), run()]
    release()

    expect(await Promise.all(calls)).toEqual([1, 1, 1])
    expect(runs).toBe(1)
  })

  it('runs again once the last run is over', async () => {
    let runs = 0
    const run = singleFlight(async () => ++runs)

    expect(await run()).toBe(1)
    expect(await run()).toBe(2)
  })

  it('runs again after a run that failed', async () => {
    let runs = 0
    const run = singleFlight(async () => {
      runs += 1
      if (runs === 1) throw new Error('first')

      return runs
    })

    await expect(run()).rejects.toThrow('first')
    expect(await run()).toBe(2)
  })
})
