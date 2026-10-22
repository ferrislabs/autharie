import type { Scenario } from './cedar-engine'
import { splitList } from './rule-form'
import { TOOL_CATALOGUE, toolId } from './tool-catalogue'

export interface SimulationDraft {
  kind: 'user' | 'agent'
  userId: string
  groups: string
  roles: string
  agent: string
  tool: string
  time: string
}

export const INITIAL_DRAFT: SimulationDraft = {
  kind: 'user',
  userId: 'user-1',
  groups: '',
  roles: '',
  agent: 'my-agent',
  tool: toolId(TOOL_CATALOGUE[0]),
  time: '',
}

export function scenarioOf(draft: SimulationDraft, now: Date): Scenario {
  const parsed = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(draft.time)
    ? new Date(`${draft.time}:00Z`)
    : null
  const time = parsed !== null && !Number.isNaN(parsed.getTime()) ? parsed : now
  const agent = draft.agent.trim() || INITIAL_DRAFT.agent

  return {
    subject:
      draft.kind === 'user'
        ? {
            kind: 'user',
            id: draft.userId.trim() || INITIAL_DRAFT.userId,
            groups: splitList(draft.groups),
            roles: splitList(draft.roles),
            agent,
          }
        : { kind: 'agent', clientId: agent },
    tool: draft.tool,
    time,
  }
}
