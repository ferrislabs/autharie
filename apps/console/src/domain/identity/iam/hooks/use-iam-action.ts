import { useCallback, useEffect, useRef, useState } from 'react'
import { runIamFlow, type IamEvent } from '../iam-flow'
import { viewOf, type IamView } from '../iam-view'
import type { IamOperation } from '../types'
import { dependencies } from './use-iam-request'
import { useIamScope } from './use-iam-scope'

export function useIamAction<T>(
  realm: string,
  operation: IamOperation,
  parse: (value: unknown) => T,
): { view: IamView<T>; run: () => void; reset: () => void } {
  const scope = useIamScope()
  const [event, setEvent] = useState<IamEvent<T> | null>(null)
  const [started, setStarted] = useState(false)
  const cancel = useRef<(() => void) | null>(null)

  useEffect(
    () => () => {
      cancel.current?.()
    },
    [],
  )

  const reset = useCallback(() => {
    cancel.current?.()
    cancel.current = null
    setEvent(null)
    setStarted(false)
  }, [])

  const run = useCallback(() => {
    cancel.current?.()
    let cancelled = false
    cancel.current = () => {
      cancelled = true
    }
    setEvent(null)
    setStarted(true)
    void runIamFlow(
      dependencies(scope, realm, operation, parse),
      () => cancelled,
      setEvent,
    )
  }, [scope, realm, operation, parse])

  return { view: viewOf(event, started), run, reset }
}
