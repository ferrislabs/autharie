import type { Tone } from '@/components/ui/status-badge'
import type { AgentPolicy, AgentServer } from './types'

export interface ServerChip {
  tone: Tone
  label: string
  detail?: string
}

export function describeServer(server: AgentServer): ServerChip {
  if (!server.enabled) return { tone: 'neutral', label: 'Off' }

  const discovery = server.discovery
  if (!discovery) return { tone: 'progress', label: 'Discovering tools' }

  switch (discovery.phase) {
    case 'Ready':
      return { tone: 'success', label: 'Ready', detail: discovery.message }
    case 'Unreachable':
      return { tone: 'danger', label: 'Unreachable', detail: discovery.message }
    case 'Refused':
      return { tone: 'danger', label: 'Refused', detail: discovery.message }
    case 'Invalid':
      return { tone: 'warning', label: 'Invalid answer', detail: discovery.message }
  }
}

export function toolCount(server: AgentServer): string | null {
  const discovery = server.discovery
  if (!discovery || discovery.phase !== 'Ready') return null
  const count = discovery.tools.length
  const more = discovery.dropped > 0 ? `, ${discovery.dropped} left out` : ''
  return `${count} ${count === 1 ? 'tool' : 'tools'}${more}`
}

export function policiesReferring(policies: AgentPolicy[], slug: string): AgentPolicy[] {
  return policies.filter((policy) => {
    const what = policy.rule?.what
    if (what) {
      return what.kind === 'tools'
        ? what.tools.some((tool) => tool.startsWith(`${slug}/`))
        : what.server === slug
    }
    return (
      policy.cedar.includes(`resource.server == "${slug}"`) ||
      policy.cedar.includes(`Tool::"${slug}/`)
    )
  })
}
