import { useMemo, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { RequiresPermission } from '@/components/layout/requires-permission'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { CAN } from '@/domain/organisations/permissions'
import { emptyFilters, type DecisionFilters } from '../../decisions'
import { useAgentScope } from '../../hooks/use-agent-scope'
import {
  useAgentDecisions,
  useAgentPolicies,
  useAgentRealms,
  useAgentServers,
  useAgentSettings,
  useDeletePolicy,
  useDeleteServer,
  useDiscoverServer,
  useSavePolicy,
  useSaveServer,
  useUpdateAgentSettings,
} from '../../hooks/use-agents'
import type { PolicyTemplateDraft } from '../../policy-templates'
import { policiesReferring } from '../../server-status'
import { catalogueOf } from '../../tool-catalogue'
import type { AgentPolicy, AgentServer } from '../../types'
import { CatalogueContext } from '../ui/catalogue-context'
import { DeletePolicyDialog } from '../ui/delete-policy-dialog'
import { PageAgents } from '../ui/page-agents'
import { PolicyCanvas } from '../ui/policy-canvas/policy-canvas'
import { PolicySheet } from '../ui/policy-sheet'
import { RemoveServerDialog } from '../ui/remove-server-dialog'
import { ServerDialog } from '../ui/server-dialog'

const NO_POLICIES: AgentPolicy[] = []
const NO_SERVERS: AgentServer[] = []

type Editing = { policy: AgentPolicy | null; initial?: PolicyTemplateDraft } | null
type EditingServer = { server: AgentServer | null } | null

function AgentsTab() {
  const scope = useAgentScope()
  const navigate = useNavigate()
  const organisationPath = useOrganisationPath()
  const { can } = useMyPermissions()

  const settings = useAgentSettings(scope)
  const realms = useAgentRealms(scope)
  const policies = useAgentPolicies(scope)
  const updateSettings = useUpdateAgentSettings(scope)
  const savePolicy = useSavePolicy(scope)
  const deletePolicy = useDeletePolicy(scope)
  const servers = useAgentServers(scope)
  const saveServer = useSaveServer(scope)
  const deleteServer = useDeleteServer(scope)
  const discoverServer = useDiscoverServer(scope)

  const [filters, setFilters] = useState<DecisionFilters>(emptyFilters)
  const decisions = useAgentDecisions(scope, filters)

  const [realmDraft, setRealmDraft] = useState<string | null>(null)
  const [editing, setEditing] = useState<Editing>(null)
  const [deleting, setDeleting] = useState<AgentPolicy | null>(null)
  const [editingServer, setEditingServer] = useState<EditingServer>(null)
  const [removingServer, setRemovingServer] = useState<AgentServer | null>(null)

  const current = settings.data
  const realm = realmDraft ?? current?.realm ?? ''
  const list = policies.data ?? NO_POLICIES
  const canManage = can(CAN.manageInstances)
  const serverList = servers.data ?? NO_SERVERS
  const catalogue = useMemo(() => catalogueOf(serverList, servers.isSuccess), [serverList, servers.isSuccess])

  const openEditor = (next: NonNullable<Editing>) => {
    savePolicy.reset()
    setEditing(next)
  }

  return (
    <CatalogueContext.Provider value={catalogue}>
      <PageAgents
        settings={current}
        isLoading={settings.isLoading}
        error={settings.error?.message}
        realms={realms.data ?? null}
        realm={realm}
        canManage={canManage}
        isSaving={updateSettings.isPending}
        saveError={updateSettings.error?.message}
        policies={list}
        policiesLoading={policies.isLoading}
        policiesError={policies.error?.message}
        busyPolicyId={savePolicy.isPending ? (savePolicy.variables?.id ?? null) : null}
        decisions={decisions.data ?? []}
        decisionsLoading={decisions.isLoading}
        decisionsError={decisions.error?.message}
        filters={filters}
        onFilters={setFilters}
        onRealmChange={setRealmDraft}
        onToggle={(enabled) =>
          updateSettings.mutate(
            { enabled, realm: realm.trim() },
            { onSuccess: () => setRealmDraft(null) },
          )
        }
        onSaveRealm={() =>
          updateSettings.mutate(
            { enabled: current?.enabled ?? false, realm: realm.trim() },
            { onSuccess: () => setRealmDraft(null) },
          )
        }
        onOpenOrganisationSettings={() => void navigate({ to: organisationPath('/settings') })}
        servers={serverList}
        serversLoading={servers.isLoading}
        serversError={servers.error?.message ?? discoverServer.error?.message}
        busyServerId={discoverServer.isPending ? (discoverServer.variables ?? null) : null}
        onAddServer={() => {
          saveServer.reset()
          setEditingServer({ server: null })
        }}
        onEditServer={(server) => {
          saveServer.reset()
          setEditingServer({ server })
        }}
        onRemoveServer={(server) => {
          deleteServer.reset()
          setRemovingServer(server)
        }}
        onRefreshServer={(server) => discoverServer.mutate(server.id)}
        accessMap={
          <PolicyCanvas
            policies={list}
            canManage={canManage}
            publisher={{
              create: (input) => savePolicy.mutateAsync({ id: null, input }),
              update: (id, input) => savePolicy.mutateAsync({ id, input }),
              remove: (id) => deletePolicy.mutateAsync(id),
            }}
            onOpenAdvanced={(policy) => openEditor({ policy })}
          />
        }
        onCreatePolicy={(initial) => openEditor({ policy: null, initial })}
        onEditPolicy={(policy) => openEditor({ policy })}
        onDeletePolicy={(policy) => {
          deletePolicy.reset()
          setDeleting(policy)
        }}
        onTogglePolicy={(policy, enabled) =>
          savePolicy.mutate({
            id: policy.id,
            input: {
              name: policy.name,
              enabled,
              cedar: policy.cedar,
              ...(policy.description ? { description: policy.description } : {}),
              ...(policy.rule ? { rule: policy.rule } : {}),
            },
          })
        }
      />

      <PolicySheet
        open={editing !== null}
        policy={editing?.policy ?? null}
        initial={editing?.initial}
        others={list}
        isSaving={savePolicy.isPending}
        error={savePolicy.error?.message}
        onClose={() => setEditing(null)}
        onSave={(input) =>
          savePolicy.mutate(
            { id: editing?.policy?.id ?? null, input },
            { onSuccess: () => setEditing(null) },
          )
        }
      />

      <DeletePolicyDialog
        policy={deleting}
        isDeleting={deletePolicy.isPending}
        error={deletePolicy.error?.message}
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          if (deleting) deletePolicy.mutate(deleting.id, { onSuccess: () => setDeleting(null) })
        }}
      />

      <ServerDialog
        open={editingServer !== null}
        server={editingServer?.server ?? null}
        taken={serverList.map((server) => server.slug)}
        isSaving={saveServer.isPending}
        error={saveServer.error?.message}
        onClose={() => setEditingServer(null)}
        onSave={(save) => saveServer.mutate(save, { onSuccess: () => setEditingServer(null) })}
      />

      <RemoveServerDialog
        server={removingServer}
        referring={removingServer ? policiesReferring(list, removingServer.slug) : NO_POLICIES}
        isRemoving={deleteServer.isPending}
        error={deleteServer.error?.message}
        onCancel={() => setRemovingServer(null)}
        onConfirm={() => {
          if (removingServer) {
            deleteServer.mutate(removingServer.id, { onSuccess: () => setRemovingServer(null) })
          }
        }}
      />
    </CatalogueContext.Provider>
  )
}

export default function PageAgentsFeature() {
  return (
    <RequiresPermission permission={CAN.manageInstances} what='manage agents'>
      <AgentsTab />
    </RequiresPermission>
  )
}
