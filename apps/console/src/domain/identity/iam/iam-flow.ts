import type { IamPoll, SealingJwk } from './types'

export const POLL_INTERVAL_MS = 2000
export const SLOW_AFTER_MS = 20_000
export const REQUEST_TIMEOUT_MS = 90_000

export interface IamFlowKeys<K> {
  privateKey: K
  publicJwk: SealingJwk
}

export interface IamFlowDeps<K, T> {
  generate: () => Promise<IamFlowKeys<K>>
  request: (key: SealingJwk) => Promise<string>
  poll: (requestId: string) => Promise<IamPoll>
  open: (privateKey: K, requestId: string, envelope: unknown) => Promise<T>
  sleep: (ms: number) => Promise<void>
  now: () => number
  statusOf: (error: unknown) => number | null
}

export type IamEvent<T> =
  | { phase: 'generating' }
  | { phase: 'requesting' }
  | { phase: 'waiting'; slow: boolean }
  | { phase: 'ready'; result: T }
  | { phase: 'failed'; message: string }

export async function runIamFlow<K, T>(
  deps: IamFlowDeps<K, T>,
  isCancelled: () => boolean,
  emit: (event: IamEvent<T>) => void,
): Promise<void> {
  const fail = (message: string) => emit({ phase: 'failed', message })

  try {
    emit({ phase: 'generating' })
    const keys = await deps.generate()
    if (isCancelled()) return

    emit({ phase: 'requesting' })
    const requestId = await deps.request(keys.publicJwk)
    if (isCancelled()) return

    const startedAt = deps.now()
    const deadline = startedAt + REQUEST_TIMEOUT_MS
    let slow = false
    emit({ phase: 'waiting', slow })

    while (deps.now() < deadline) {
      await deps.sleep(POLL_INTERVAL_MS)
      if (isCancelled()) return

      if (!slow && deps.now() - startedAt >= SLOW_AFTER_MS) {
        slow = true
        emit({ phase: 'waiting', slow })
      }

      let answer: IamPoll
      try {
        answer = await deps.poll(requestId)
      } catch (error) {
        if (isCancelled()) return
        const status = deps.statusOf(error)
        if (status === 410) return fail('The answer was already delivered. Reload to ask again.')
        if (status === 404) return fail('The request expired before the answer arrived. Try again.')
        throw error
      }
      if (isCancelled()) return

      if (answer.state === 'ready') {
        const result = await deps.open(keys.privateKey, requestId, answer.envelope)
        if (!isCancelled()) emit({ phase: 'ready', result })
        return
      }
    }
    fail('The data plane did not answer in time. Try again.')
  } catch (error) {
    if (isCancelled()) return
    fail(error instanceof Error ? error.message : 'Something went wrong.')
  }
}
