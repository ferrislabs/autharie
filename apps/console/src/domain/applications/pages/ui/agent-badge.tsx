import { Link } from '@tanstack/react-router'
import { StatusBadge } from '@/components/ui/status-badge'

export function AgentBadge({ agentsPath }: { agentsPath: string }) {
  return (
    <Link to={agentsPath} title='Registered from the Agents tab'>
      <StatusBadge tone='accent' dot={false}>
        Agent
      </StatusBadge>
    </Link>
  )
}
