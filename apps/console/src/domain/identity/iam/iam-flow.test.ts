import { describe, expect, it, vi } from 'vitest'
import {
  POLL_INTERVAL_MS,
  REQUEST_TIMEOUT_MS,
  SLOW_AFTER_MS,
  runIamFlow,
  type IamEvent,
  type IamFlowDeps,
} from './iam-flow'
import type { IamPoll } from './types'

const KEY = { kty: 'RSA', n: 'n', e: 'AQAB', alg: 'RSA-OAEP-256' } as const
const READY: IamPoll = { state: 'ready', envelope: { v: 1 } }

function last<T>(items: T[]): T | undefined {
  return items[items.length - 1]
}

function failure(status: number, message = 'nope') {
  return Object.assign(new Error(message), { status })
}

function setup(
  polls: Array<IamPoll | Error>,
  overrides: Partial<IamFlowDeps<string, { items: number[] }>> = {},
) {
  let clock = 1_000
  const queue = [...polls]
  const events: IamEvent<{ items: number[] }>[] = []
  const deps: IamFlowDeps<string, { items: number[] }> = {
    generate: vi.fn(async () => ({ privateKey: 'private', publicJwk: KEY })),
    request: vi.fn(async () => 'request-1'),
    poll: vi.fn(async () => {
      const next = queue.shift() ?? ({ state: 'pending' } as IamPoll)
      if (next instanceof Error) throw next
      return next
    }),
    open: vi.fn(async () => ({ items: [1] })),
    sleep: vi.fn(async (ms: number) => {
      clock += ms
    }),
    now: () => clock,
    statusOf: (error) => (error as { status?: number }).status ?? null,
    ...overrides,
  }
  return {
    deps,
    events,
    emit: (event: IamEvent<{ items: number[] }>) => events.push(event),
    elapsed: () => clock - 1_000,
  }
}

describe('the IAM flow', () => {
  it('sends only the public key, waits while pending, then opens the envelope for its request', async () => {
    const { deps, events, emit } = setup([{ state: 'pending' }, { state: 'pending' }, READY])

    await runIamFlow(deps, () => false, emit)

    expect(deps.request).toHaveBeenCalledWith(KEY)
    expect(deps.poll).toHaveBeenCalledTimes(3)
    expect(deps.poll).toHaveBeenCalledWith('request-1')
    expect(deps.open).toHaveBeenCalledWith('private', 'request-1', { v: 1 })
    expect(events.map((event) => event.phase)).toEqual([
      'generating',
      'requesting',
      'waiting',
      'ready',
    ])
    expect(last(events)).toEqual({ phase: 'ready', result: { items: [1] } })
  })

  it('polls every two seconds', async () => {
    const { deps, emit } = setup([{ state: 'pending' }, READY])

    await runIamFlow(deps, () => false, emit)

    expect(POLL_INTERVAL_MS).toBe(2000)
    expect(deps.sleep).toHaveBeenCalledWith(POLL_INTERVAL_MS)
  })

  it('says it is taking longer than usual once, after twenty seconds', async () => {
    const pending = Array.from({ length: 12 }, () => ({ state: 'pending' }) as IamPoll)
    const { deps, events, emit } = setup([...pending, READY])

    await runIamFlow(deps, () => false, emit)

    const waiting = events.filter((event) => event.phase === 'waiting')
    expect(SLOW_AFTER_MS).toBe(20_000)
    expect(waiting).toEqual([
      { phase: 'waiting', slow: false },
      { phase: 'waiting', slow: true },
    ])
    expect(events.indexOf(waiting[1])).toBeGreaterThan(events.indexOf(waiting[0]))
  })

  it('does not say it before twenty seconds have passed', async () => {
    const pending = Array.from({ length: 5 }, () => ({ state: 'pending' }) as IamPoll)
    const { deps, events, emit } = setup([...pending, READY])

    await runIamFlow(deps, () => false, emit)

    expect(events.some((event) => event.phase === 'waiting' && event.slow)).toBe(false)
  })

  it('gives up after ninety seconds with a message', async () => {
    const { deps, events, emit, elapsed } = setup([])

    await runIamFlow(deps, () => false, emit)

    expect(REQUEST_TIMEOUT_MS).toBe(90_000)
    expect(last(events)).toEqual({
      phase: 'failed',
      message: 'The data plane did not answer in time. Try again.',
    })
    expect(elapsed()).toBeLessThanOrEqual(REQUEST_TIMEOUT_MS + POLL_INTERVAL_MS)
    expect(vi.mocked(deps.poll).mock.calls.length).toBeLessThanOrEqual(
      REQUEST_TIMEOUT_MS / POLL_INTERVAL_MS,
    )
    expect(deps.open).not.toHaveBeenCalled()
  })

  it('says the answer was already delivered on a 410', async () => {
    const { deps, events, emit } = setup([failure(410)])

    await runIamFlow(deps, () => false, emit)

    expect(last(events)).toMatchObject({ phase: 'failed' })
    expect((last(events) as { message: string }).message).toContain('already delivered')
    expect(deps.open).not.toHaveBeenCalled()
  })

  it('says the request expired on a 404', async () => {
    const { deps, events, emit } = setup([failure(404)])

    await runIamFlow(deps, () => false, emit)

    expect((last(events) as { message: string }).message).toContain('expired')
    expect(deps.open).not.toHaveBeenCalled()
  })

  it('shows what the platform said when the request failed with a 422', async () => {
    const { deps, events, emit } = setup([failure(422, 'realm "ghost" was not found')])

    await runIamFlow(deps, () => false, emit)

    expect(last(events)).toEqual({ phase: 'failed', message: 'realm "ghost" was not found' })
    expect(deps.poll).toHaveBeenCalledTimes(1)
  })

  it('reports a request that is refused, such as the eleventh in flight', async () => {
    const { deps, events, emit } = setup([], {
      request: vi.fn(async () => {
        throw failure(429, 'too many requests in flight')
      }),
    })

    await runIamFlow(deps, () => false, emit)

    expect(last(events)).toEqual({ phase: 'failed', message: 'too many requests in flight' })
    expect(deps.poll).not.toHaveBeenCalled()
  })

  it('fails with a message when the envelope cannot be opened', async () => {
    const { deps, events, emit } = setup([READY], {
      open: vi.fn(async () => {
        throw new Error('The answer could not be opened.')
      }),
    })

    await runIamFlow(deps, () => false, emit)

    expect(last(events)).toEqual({ phase: 'failed', message: 'The answer could not be opened.' })
  })

  it('drops everything when cancelled while waiting: no further poll, nothing opened, no event', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([READY], {
      sleep: vi.fn(async () => {
        cancelled = true
      }),
    })

    await runIamFlow(deps, () => cancelled, emit)

    expect(deps.poll).not.toHaveBeenCalled()
    expect(deps.open).not.toHaveBeenCalled()
    expect(events.map((event) => event.phase)).toEqual(['generating', 'requesting', 'waiting'])
  })

  it('does not ask for anything when cancelled before the key exists', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([], {
      generate: vi.fn(async () => {
        cancelled = true
        return { privateKey: 'private', publicJwk: KEY }
      }),
    })

    await runIamFlow(deps, () => cancelled, emit)

    expect(deps.request).not.toHaveBeenCalled()
    expect(events.map((event) => event.phase)).toEqual(['generating'])
  })

  it('opens nothing when cancelled while the poll was in flight', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([], {
      poll: vi.fn(async () => {
        cancelled = true
        return READY
      }),
    })

    await runIamFlow(deps, () => cancelled, emit)

    expect(deps.open).not.toHaveBeenCalled()
    expect(events.some((event) => event.phase === 'ready')).toBe(false)
  })

  it('does not report a result that arrives after cancellation', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([READY], {
      open: vi.fn(async () => {
        cancelled = true
        return { items: [1] }
      }),
    })

    await runIamFlow(deps, () => cancelled, emit)

    expect(events.some((event) => event.phase === 'ready')).toBe(false)
  })

  it('does not report a failure after cancellation either', async () => {
    let cancelled = false
    const { deps, events, emit } = setup([], {
      request: vi.fn(async () => {
        cancelled = true
        throw failure(500, 'late')
      }),
    })

    await runIamFlow(deps, () => cancelled, emit)

    expect(events.some((event) => event.phase === 'failed')).toBe(false)
  })
})
