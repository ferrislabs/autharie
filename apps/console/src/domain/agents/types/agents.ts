import type { RuleModel } from './rule'

export type AgentPhase = 'Applying' | 'Ready' | 'Failed'

export interface AgentStatus {
  phase: AgentPhase
  gateway_client_ready: boolean
  message?: string
  updated_at: string
}

export interface AgentSettings {
  enabled: boolean
  realm: string
  mcp_url?: string
  organisation_mcp_enabled: boolean
  status?: AgentStatus
}

export interface AgentSettingsInput {
  enabled: boolean
  realm: string
}

export type PolicyPhase = 'Pending' | 'Applied' | 'Invalid' | 'Failed'

export interface AgentPolicy {
  id: string
  name: string
  description?: string
  enabled: boolean
  rule?: RuleModel
  cedar: string
  phase?: PolicyPhase
  message?: string
  created_at: string
  updated_at: string
}

export interface AgentPolicyInput {
  name: string
  description?: string
  enabled: boolean
  rule?: RuleModel
  cedar: string
}

export type ServerPhase = 'Ready' | 'Unreachable' | 'Refused' | 'Invalid'

export interface DiscoveredTool {
  name: string
  description?: string
  read_only: boolean
  destructive: boolean
}

export interface ServerDiscovery {
  phase: ServerPhase
  tools: DiscoveredTool[]
  dropped: number
  message?: string
  discovered_at: string
}

export interface AgentServer {
  id: string
  slug: string
  name: string
  url: string
  enabled: boolean
  mcp_url?: string
  discovery?: ServerDiscovery
  created_at: string
  updated_at: string
}

export interface AgentServerCreate {
  slug: string
  name: string
  url: string
  enabled?: boolean
}

export interface AgentServerChange {
  name: string
  url: string
  enabled: boolean
}

export interface AgentDecision {
  kind: 'agent_decision'
  time: string
  server: string
  method: string
  tool?: string
  subject_type: string
  subject_id: string
  agent: string
  delegated: boolean
  decision: string | boolean
  policies: string[]
  reason?: string
  request_id: string
  deployment_id: string
}

export interface DecisionQuery {
  from?: string
  to?: string
  decision?: 'allow' | 'deny'
  tool?: string
  subject?: string
  limit?: number
}

export interface Scope {
  organisationId: string | null
  deploymentId: string | null
}
