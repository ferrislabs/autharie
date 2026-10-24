import { useCallback, useEffect, useState } from 'react'
import { ApiRequestError } from '@/api/api.fetch'
import { runIamFlow, type IamEvent, type IamFlowDeps } from '../iam-flow'
import { viewOf, type IamView } from '../iam-view'
import { generateSealingKeyPair, openEnvelope } from '../sealing'
import { askIam, fetchIamResult } from '../services/iam.service'
import type { IamOperation, IamScope } from '../types'
import { useIamScope } from './use-iam-scope'

function dependencies<T>(
  scope: IamScope,
  realm: string,
  operation: IamOperation,
  parse: (value: unknown) => T,
): IamFlowDeps<CryptoKey, T> {
  return {
    generate: generateSealingKeyPair,
    request: (sealingKey) =>
      askIam(scope, { realm, operation, sealing_key: sealingKey }),
    poll: (requestId) => fetchIamResult(scope, requestId),
    open: async (privateKey, requestId, envelope) =>
      parse(JSON.parse(await openEnvelope(privateKey, requestId, envelope))),
    sleep: (ms) => new Promise((resolve) => window.setTimeout(resolve, ms)),
    now: () => Date.now(),
    statusOf: (error) => (error instanceof ApiRequestError ? error.status : null),
  }
}

interface Outcome<T> {
  key: string
  event: IamEvent<T>
}

export function useIamRequest<T>(
  realm: string,
  operation: IamOperation | null,
  parse: (value: unknown) => T,
): { view: IamView<T>; retry: () => void } {
  const scope = useIamScope()
  const [attempt, setAttempt] = useState(0)
  const [outcome, setOutcome] = useState<Outcome<T> | null>(null)

  const enabled = operation !== null && scope.organisationId !== null && scope.deploymentId !== null
  const key = enabled ? JSON.stringify([scope, realm, operation, attempt]) : null

  useEffect(() => {
    if (key === null || operation === null) return

    let cancelled = false
    void runIamFlow(
      dependencies(scope, realm, operation, parse),
      () => cancelled,
      (event) => setOutcome({ key, event }),
    )

    return () => {
      cancelled = true
    }
  }, [key, scope, realm, operation, parse])

  const retry = useCallback(() => setAttempt((count) => count + 1), [])
  const event = outcome !== null && outcome.key === key ? outcome.event : null

  return { view: viewOf(event, enabled), retry }
}
