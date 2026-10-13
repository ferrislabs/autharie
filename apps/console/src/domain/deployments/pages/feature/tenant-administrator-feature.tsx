import { useState } from 'react'
import { useIssueTenantAdministrator, useRevealTenantAdministrator } from '@/api/deployment.api'
import {
  ADMINISTRATOR_COPY,
  type RevealedAdministrator,
  type TenantAdministratorView,
} from '../../tenant-realm'
import { TenantAdministratorPanel } from '../ui/components/tenant-administrator-panel'

interface Props {
  organisationId?: string
  deploymentId: string
  administrator: TenantAdministratorView
}

function readRevealed(value: unknown): RevealedAdministrator | undefined {
  const data = (value as { data?: { username?: unknown; password?: unknown } } | null)?.data
  if (typeof data?.username !== 'string' || typeof data.password !== 'string') return undefined
  return { username: data.username, password: data.password }
}

export function TenantAdministratorFeature({ organisationId, deploymentId, administrator }: Props) {
  const issue = useIssueTenantAdministrator()
  const reveal = useRevealTenantAdministrator()
  const [revealed, setRevealed] = useState<RevealedAdministrator>()
  const [unreadable, setUnreadable] = useState(false)

  const path = organisationId
    ? { organisation_id: organisationId, deployment_id: deploymentId }
    : undefined

  const onIssue = () => {
    if (!path) return
    setRevealed(undefined)
    setUnreadable(false)
    issue.mutate({ path })
  }

  const onReveal = () => {
    if (!path) return
    setUnreadable(false)
    reveal.mutate(
      { path },
      {
        onSuccess: async (answer) => {
          const parsed = await answer.json().then(readRevealed, () => undefined)
          if (parsed) setRevealed(parsed)
          else setUnreadable(true)
        },
      }
    )
  }

  const failure = unreadable
    ? ADMINISTRATOR_COPY.unreadable
    : [issue.error, reveal.error].find((error) => error instanceof Error)?.message

  return (
    <TenantAdministratorPanel
      administrator={administrator}
      revealed={revealed}
      issuing={issue.isPending}
      revealing={reveal.isPending}
      error={failure}
      onIssue={onIssue}
      onReveal={onReveal}
    />
  )
}
