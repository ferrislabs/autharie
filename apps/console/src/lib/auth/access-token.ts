import type { User } from 'oidc-client-ts'
import { useAuthStore } from '@/stores/auth'
import { renewalFor, type StoredSession } from './renewal'
import { renewSession } from './session'
import { getUserManager } from './user-manager'

/**
 * How long a request waits for a renewal before it leaves with what it has.
 *
 * A renewal that never answers -- a lock another tab holds, an identity
 * provider that is down -- must not freeze every call the console makes. The
 * platform's refusal is a better outcome than a request that never leaves.
 */
export const RENEWAL_WAIT_MS = 10_000

export interface TokenSource {
  stored(): { accessToken: string | null; session: StoredSession | null }
  renew(): Promise<User | null>
  adopt(user: User): void
}

/** When the access token itself says it expires, in epoch seconds. */
export function expiryOf(token: string): number | undefined {
  const payload = token.split('.')[1]
  if (!payload) return undefined

  try {
    const base64 = payload.replace(/-/g, '+').replace(/_/g, '/')
    const json = atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, '='))
    const exp = (JSON.parse(json) as { exp?: unknown }).exp

    return typeof exp === 'number' ? exp : undefined
  } catch {
    return undefined
  }
}

function earliest(...values: (number | undefined)[]): number | undefined {
  const known = values.filter((value): value is number => value !== undefined)

  return known.length > 0 ? Math.min(...known) : undefined
}

function within<T>(work: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('renewal took too long')), ms)

    work.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(error)
      }
    )
  })
}

/**
 * The access token to send now, renewed first when it is about to expire.
 *
 * The timer in `AuthLayout` renews a session ahead of time, but a timer is
 * only a plan: a tab that sat in the background, a renewal another tab did
 * that this one never heard of, a token that expires earlier than the session
 * says. Whatever the reason, the request that leaves is the one that finds
 * out, so it checks the token it is about to send -- by the expiry the token
 * carries, not by the one stored next to it.
 *
 * When there is nothing to renew with, or the renewal fails, the token goes
 * as it is and the platform gives the answer.
 */
export async function accessTokenFor(
  source: TokenSource,
  now: number,
  wait: number = RENEWAL_WAIT_MS
): Promise<string | null> {
  const { accessToken, session } = source.stored()
  if (!accessToken) return null

  const due = renewalFor(
    {
      expiresAt: earliest(expiryOf(accessToken), session?.expiresAt),
      refreshToken: session?.refreshToken,
    },
    now
  )
  if (due !== 'exchange') return accessToken

  try {
    const user = await within(source.renew(), wait)
    if (!user) return accessToken

    source.adopt(user)

    return user.access_token
  } catch {
    return accessToken
  }
}

/** Runs `work` once at a time: callers that arrive meanwhile share its result. */
export function singleFlight<T>(work: () => Promise<T>): () => Promise<T> {
  let running: Promise<T> | null = null

  return () => {
    running ??= work().finally(() => {
      running = null
    })

    return running
  }
}

const browserSource: TokenSource = {
  stored: () => {
    const { accessToken, user } = useAuthStore.getState()

    return {
      accessToken,
      session: user ? { expiresAt: user.expires_at, refreshToken: user.refresh_token } : null,
    }
  },
  renew: async () => {
    const manager = getUserManager()

    return manager ? renewSession(manager) : null
  },
  adopt: (user) => useAuthStore.getState().setUser(user),
}

export const currentAccessToken = singleFlight(() =>
  accessTokenFor(browserSource, Math.floor(Date.now() / 1000))
)
