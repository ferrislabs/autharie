import { useCallback, useEffect, useRef, useState } from 'react'
import { ApiRequestError } from '@/api/api.fetch'
import { generateSealingKeyPair, openSealed } from '../credential-crypto'
import { runCredentialFlow, type FlowDeps, type FlowEvent } from '../credential-flow'
import { fetchCredential } from '../services/registry.service'
import type { Scope, SealingJwk } from '../types'

function dependencies(
  scope: Scope,
  request: (key: SealingJwk) => Promise<string>,
): FlowDeps<CryptoKey> {
  return {
    generate: generateSealingKeyPair,
    request,
    poll: (requestId) => fetchCredential(scope, requestId),
    open: openSealed,
    sleep: (ms) => new Promise((resolve) => window.setTimeout(resolve, ms)),
    now: () => Date.now(),
    statusOf: (error) => (error instanceof ApiRequestError ? error.status : null),
  }
}

export function useCredentialFlow(scope: Scope) {
  const [event, setEvent] = useState<FlowEvent | null>(null)
  const generation = useRef(0)

  const reset = useCallback(() => {
    generation.current += 1
    setEvent(null)
  }, [])

  const start = useCallback(
    (request: (key: SealingJwk) => Promise<string>) => {
      generation.current += 1
      const run = generation.current
      void runCredentialFlow(
        dependencies(scope, request),
        () => generation.current !== run,
        setEvent,
      )
    },
    [scope],
  )

  useEffect(() => reset, [reset])

  return { event, start, reset }
}
