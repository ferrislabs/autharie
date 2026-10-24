import type { ApplicationSummary } from './types'

const AGENT_PREFIX = 'agent-'

export function isAgentApplication(application: Pick<ApplicationSummary, 'client_id'>): boolean {
  return application.client_id.startsWith(AGENT_PREFIX)
}
