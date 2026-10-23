import { useEffect, useState } from 'react'
import type { UseQueryResult } from '@tanstack/react-query'
import type { AgentAction } from '../../agent-actions'
import { useCredentialFlow } from '../../hooks/use-credential-flow'
import { useRegistryActions } from '../../hooks/use-registry'
import type { RegistryFilter } from '../../registry'
import {
  createAutonomousAgent,
  rotateAutonomousSecret,
} from '../../services/registry.service'
import type { AgentClient, Registry, Scope } from '../../types'
import { AgentRegistry } from '../ui/agent-registry'
import { AutonomousAgentDialog, type AutonomousMode } from '../ui/autonomous-agent-dialog'
import { ConfirmAgentDialog } from '../ui/confirm-agent-dialog'

interface Props {
  scope: Scope
  canManage: boolean
  registry: UseQueryResult<Registry>
}

export function AgentRegistryFeature({ scope, canManage, registry }: Props) {
  const actions = useRegistryActions(scope)
  const flow = useCredentialFlow(scope)
  const [filter, setFilter] = useState<RegistryFilter>('all')
  const [confirming, setConfirming] = useState<AgentAction | null>(null)
  const [mode, setMode] = useState<AutonomousMode | null>(null)
  const [busyClient, setBusyClient] = useState<string | null>(null)

  const ready = flow.event?.phase === 'ready'
  const { refresh } = actions
  useEffect(() => {
    if (ready) void refresh()
  }, [ready, refresh])

  const run = (client: AgentClient, mutate: (clientId: string) => Promise<unknown>) => {
    setBusyClient(client.client_id)
    void mutate(client.client_id).finally(() => setBusyClient(null))
  }

  const closeDialog = () => {
    flow.reset()
    setMode(null)
  }

  const confirm = () => {
    if (!confirming) return
    const mutation = confirming.kind === 'block' ? actions.block : actions.remove
    mutation.mutate(confirming.client.client_id, { onSuccess: () => setConfirming(null) })
  }

  const confirmation = confirming?.kind === 'block' ? actions.block : actions.remove

  return (
    <>
      <AgentRegistry
        registry={registry.data}
        isLoading={registry.isLoading}
        error={registry.error?.message ?? actions.approvalRequired.error?.message}
        canManage={canManage}
        filter={filter}
        approvalSaving={actions.approvalRequired.isPending}
        busyClient={busyClient}
        now={new Date(registry.dataUpdatedAt)}
        onFilter={setFilter}
        onApprovalRequired={(required) => actions.approvalRequired.mutate(required)}
        onApprove={(client) => run(client, actions.approve.mutateAsync)}
        onUnblock={(client) => run(client, actions.unblock.mutateAsync)}
        onBlock={(client) => {
          actions.block.reset()
          setConfirming({ kind: 'block', client })
        }}
        onDelete={(client) => {
          actions.remove.reset()
          setConfirming({ kind: 'delete', client })
        }}
        onRotate={(client) => setMode({ kind: 'rotate', client })}
        onNewAutonomous={() => setMode({ kind: 'create' })}
      />

      <ConfirmAgentDialog
        action={confirming}
        isBusy={confirmation.isPending}
        error={confirmation.error?.message}
        onConfirm={confirm}
        onCancel={() => setConfirming(null)}
      />

      <AutonomousAgentDialog
        mode={mode}
        event={flow.event}
        onCreate={(name, description) =>
          flow.start((key) =>
            createAutonomousAgent(scope, {
              name,
              ...(description ? { description } : {}),
              sealing_key: key,
            }),
          )
        }
        onRotate={() => {
          if (mode?.kind === 'rotate') {
            const clientId = mode.client.client_id
            flow.start((key) => rotateAutonomousSecret(scope, clientId, key))
          }
        }}
        onClose={closeDialog}
      />
    </>
  )
}
