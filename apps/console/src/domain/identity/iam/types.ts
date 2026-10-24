import type { SealingJwk } from '@/domain/agents/types'

export type { SealingJwk }

export interface IamScope {
  organisationId: string | null
  deploymentId: string | null
}

export type IamOperation =
  | { type: 'applications.list'; page?: number; limit?: number; search?: string }
  | { type: 'applications.get'; id: string }

export interface IamRequestBody {
  realm: string
  operation: IamOperation
  sealing_key: SealingJwk
}

export type IamPoll = { state: 'pending' } | { state: 'ready'; envelope: unknown }
