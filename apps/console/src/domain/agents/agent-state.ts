import type { Tone } from '@/components/ui/status-badge'
import type { AgentSettings } from './types'

export const ORGANISATION_OFF_NOTE =
  'AI access is off for this organisation, so agents cannot reach this deployment. Turn it on in the organisation settings.'

export type AgentsKind = 'organisation-off' | 'disabled' | 'applying' | 'ready' | 'failed'

export interface Badge {
  tone: Tone
  label: string
}

export interface AgentsState {
  kind: AgentsKind
  switchDisabled: boolean
  callout: string | null
  badge: Badge
  gateway: string
  message: string | null
}

export function describeAgents(settings: AgentSettings, canManage: boolean): AgentsState {
  const locked = !canManage
  const status = settings.status

  if (!settings.organisation_mcp_enabled) {
    return {
      kind: 'organisation-off',
      switchDisabled: true,
      callout: ORGANISATION_OFF_NOTE,
      badge: { tone: 'neutral', label: 'Off for the organisation' },
      gateway: 'Not applicable',
      message: null,
    }
  }

  if (!settings.enabled) {
    return {
      kind: 'disabled',
      switchDisabled: locked,
      callout: null,
      badge: { tone: 'neutral', label: 'Off' },
      gateway: 'Not applicable',
      message: null,
    }
  }

  const gateway = status?.gateway_client_ready ? 'Ready' : 'Not ready'

  if (!status) {
    return {
      kind: 'applying',
      switchDisabled: locked,
      callout: null,
      badge: { tone: 'progress', label: 'Waiting for the data plane' },
      gateway,
      message: null,
    }
  }

  switch (status.phase) {
    case 'Ready':
      return {
        kind: 'ready',
        switchDisabled: locked,
        callout: null,
        badge: { tone: 'success', label: 'Ready' },
        gateway,
        message: status.message ?? null,
      }
    case 'Failed':
      return {
        kind: 'failed',
        switchDisabled: locked,
        callout: null,
        badge: { tone: 'danger', label: 'Failed' },
        gateway,
        message: status.message ?? null,
      }
    case 'Applying':
      return {
        kind: 'applying',
        switchDisabled: locked,
        callout: null,
        badge: { tone: 'progress', label: 'Applying' },
        gateway,
        message: status.message ?? null,
      }
  }
}
