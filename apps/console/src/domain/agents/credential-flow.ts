import type { CredentialPoll, SealedCredential, SealingJwk } from './types'

export const POLL_INTERVAL_MS = 2000
export const CREDENTIAL_LIFETIME_MS = 15 * 60 * 1000

export interface FlowKeys<K> {
  privateKey: K
  publicJwk: SealingJwk
}

export interface FlowDeps<K> {
  generate: () => Promise<FlowKeys<K>>
  request: (key: SealingJwk) => Promise<string>
  poll: (requestId: string) => Promise<CredentialPoll>
  open: (privateKey: K, sealed: string) => Promise<string>
  sleep: (ms: number) => Promise<void>
  now: () => number
  statusOf: (error: unknown) => number | null
}

export interface Revealed {
  clientId: string
  secret: string
  tokenUrl: string
  audience: string
}

export type FlowEvent =
  | { phase: 'generating' }
  | { phase: 'requesting' }
  | { phase: 'waiting' }
  | { phase: 'ready'; revealed: Revealed }
  | { phase: 'failed'; message: string }

function reveal(credential: SealedCredential, secret: string): Revealed {
  return {
    clientId: credential.client_id,
    secret,
    tokenUrl: credential.token_url,
    audience: credential.audience,
  }
}

export async function runCredentialFlow<K>(
  deps: FlowDeps<K>,
  isCancelled: () => boolean,
  emit: (event: FlowEvent) => void,
): Promise<void> {
  const fail = (message: string) => emit({ phase: 'failed', message })

  try {
    emit({ phase: 'generating' })
    const keys = await deps.generate()
    if (isCancelled()) return

    emit({ phase: 'requesting' })
    const requestId = await deps.request(keys.publicJwk)
    if (isCancelled()) return

    emit({ phase: 'waiting' })
    const deadline = deps.now() + CREDENTIAL_LIFETIME_MS
    while (deps.now() < deadline) {
      await deps.sleep(POLL_INTERVAL_MS)
      if (isCancelled()) return

      let answer: CredentialPoll
      try {
        answer = await deps.poll(requestId)
      } catch (error) {
        const status = deps.statusOf(error)
        if (status === 410) return fail('The secret was already collected. Rotate it to get a new one.')
        if (status === 404) return fail('The request expired before the secret arrived. Try again.')
        throw error
      }
      if (isCancelled()) return
      if (answer.state === 'ready') {
        const secret = await deps.open(keys.privateKey, answer.credential.sealed)
        if (!isCancelled()) emit({ phase: 'ready', revealed: reveal(answer.credential, secret) })
        return
      }
    }
    fail('The data plane did not answer in time. Try again.')
  } catch (error) {
    fail(error instanceof Error ? error.message : 'Something went wrong.')
  }
}
