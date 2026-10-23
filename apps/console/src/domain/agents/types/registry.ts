export type ClientSource = 'dynamic' | 'metadata_document' | 'autharie' | 'seen'
export type ClientKind = 'delegated' | 'autonomous'
export type AccessState = 'approved' | 'pending' | 'blocked'

export interface AgentClient {
  client_id: string
  name?: string
  source: ClientSource
  kind: ClientKind
  enabled?: boolean
  access: AccessState
  created_at?: string
  last_seen_at?: string
}

export interface Registry {
  approval_required: boolean
  data: AgentClient[]
  truncated: boolean
}

export interface SealingJwk {
  kty: 'RSA'
  n: string
  e: string
  alg: 'RSA-OAEP-256'
}

export interface AutonomousInput {
  name: string
  description?: string
  sealing_key: SealingJwk
}

export interface SealedCredential {
  client_id: string
  sealed: string
  token_url: string
  audience: string
}

export type CredentialPoll =
  | { state: 'pending' }
  | { state: 'ready'; credential: SealedCredential }
