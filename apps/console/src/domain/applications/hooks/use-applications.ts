import { useMemo } from 'react'
import { useIamRequest } from '@/domain/identity/iam/hooks/use-iam-request'
import { listOperation, type ListState } from '../list-state'
import { parseApplicationDetail, parseApplicationList } from '../services/application-result'

export function useApplicationList(realm: string, state: ListState) {
  const operation = useMemo(() => listOperation(state), [state])

  return useIamRequest(realm, operation, parseApplicationList)
}

export function useApplication(realm: string, id: string) {
  const operation = useMemo(() => ({ type: 'applications.get' as const, id }), [id])

  return useIamRequest(realm, operation, parseApplicationDetail)
}
