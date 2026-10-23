import { describe, expect, it, vi } from 'vitest'
import {
  CREDENTIAL_LIFETIME_MS,
  POLL_INTERVAL_MS,
  runCredentialFlow,
  type FlowDeps,
  type FlowEvent,
} from './credential-flow'
import type { CredentialPoll } from './types'

const KEY = { kty: 'RSA', n: 'n', e: 'AQAB', alg: 'RSA-OAEP-256' } as const
function last<T>(items: T[]): T | undefined {
  return items[items.length - 1]
}

const READY: CredentialPoll = {
  state: 'ready',
  credential: {
    client_id: 'agent-x-abc123',
    sealed: 'sealed',
    token_url: 'https://auth.test/token',
    audience: 'https://auth.test/mcp',
  },
}

function setup(polls: Array<CredentialPoll | Error>, overrides: Partial<FlowDeps<string>> = {}) {
  let clock = 0
  const queue = [...polls]
  const events: FlowEvent[] = []
  const deps: FlowDeps<string> = {
    generate: vi.fn(async () => ({ privateKey: 'private', publicJwk: KEY })),
    request: vi.fn(async () => 'request-1'),
    poll: vi.fn(async () => {
      const next = queue.shift() ?? ({ state: 'pending' } as CredentialPoll)
      if (next instanceof Error) throw next
      return next
    }),
    open: vi.fn(async () => 'the-secret'),
    sleep: vi.fn(async (ms: number) => {
      clock += ms
    }),
    now: () => clock,
    statusOf: (error) => (error as { status?: number }).status ?? null,
    ...overrides,
  }
  return { deps, events, emit: (event: FlowEvent) => events.push(event) }
}

describe('the credential flow', () => {
  it('sends only the public key, waits, decrypts and reveals once', async () => {
    const { deps, events, emit } = setup([{ state: 'pending' }, { state: 'pending' }, READY])

    await runCredentialFlow(deps, () => false, emit)

    expect(deps.request).toHaveBeenCalledWith(KEY)
    expect(deps.poll).toHaveBeenCalledTimes(3)
    expect(deps.open).toHaveBeenCalledWith('private', 'sealed')
    expect(events.map((event) => event.phase)).toEqual([
      'generating',
      'requesting',
      'waiting',
      'ready',
    ])
    expect(last(events)).toEqual({
      phase: 'ready',
      revealed: {
        clientId: 'agent-x-abc123',
        secret: 'the-secret',
        tokenUrl: 'https://auth.test/token',
        audience: 'https://auth.test/mcp',
      },
    })
  })

  it('polls every two seconds', async () => {
    const { deps, emit } = setup([{ state: 'pending' }, READY])

    await runCredentialFlow(deps, () => false, emit)

    expect(deps.sleep).toHaveBeenCalledWith(POLL_INTERVAL_MS)
  })

  it('says the secret was already collected on a 410', async () => {
    const { deps, events, emit } = setup([Object.assign(new Error('gone'), { status: 410 })])

    await runCredentialFlow(deps, () => false, emit)

    expect(last(events)).toMatchObject({ phase: 'failed' })
    expect((last(events) as { message: string }).message).toContain('already collected')
    expect(deps.open).not.toHaveBeenCalled()
  })

  it('says the request expired on a 404', async () => {
    const { deps, events, emit } = setup([Object.assign(new Error('missing'), { status: 404 })])

    await runCredentialFlow(deps, () => false, emit)

    expect((last(events) as { message: string }).message).toContain('expired')
  })

  it('gives up after the fifteen minutes the credential lives', async () => {
    const { deps, events, emit } = setup([])

    await runCredentialFlow(deps, () => false, emit)

    expect(last(events)).toMatchObject({ phase: 'failed' })
    expect(vi.mocked(deps.poll).mock.calls.length).toBeLessThanOrEqual(
      CREDENTIAL_LIFETIME_MS / POLL_INTERVAL_MS,
    )
  })

  it('reports a request that is refused, such as a sealing key the server rejects', async () => {
    const { deps, events, emit } = setup([], {
      request: vi.fn(async () => {
        throw new Error('the key must be at least 2048 bits')
      }),
    })

    await runCredentialFlow(deps, () => false, emit)

    expect(last(events)).toEqual({
      phase: 'failed',
      message: 'the key must be at least 2048 bits',
    })
    expect(deps.poll).not.toHaveBeenCalled()
  })

  it('drops everything when the dialog is closed: no further event, nothing decrypted', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([READY], {
      sleep: vi.fn(async () => {
        cancelled = true
      }),
    })

    await runCredentialFlow(deps, () => cancelled, emit)

    expect(deps.open).not.toHaveBeenCalled()
    expect(events.some((event) => event.phase === 'ready')).toBe(false)
  })
})
