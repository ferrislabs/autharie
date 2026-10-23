import { displayName } from './registry'
import type { AgentClient } from './types'

export interface AgentAction {
  kind: 'block' | 'delete'
  client: AgentClient
}

const COPY = {
  block: {
    title: (name: string) => `Block ${name}?`,
    body: 'The gateway refuses every call from this agent before any policy is read, and its client is disabled in the realm so it gets no new token. You can unblock it later.',
    confirm: 'Block agent',
  },
  delete: {
    title: (name: string) => `Delete ${name}?`,
    body: 'The agent’s client is deleted from the realm and it is forgotten here. A dynamic or metadata-document client can register again and will then be pending.',
    confirm: 'Delete agent',
  },
}

export function confirmCopy(action: AgentAction): { title: string; body: string; confirm: string } {
  const copy = COPY[action.kind]
  return {
    title: copy.title(displayName(action.client)),
    body: copy.body,
    confirm: copy.confirm,
  }
}
