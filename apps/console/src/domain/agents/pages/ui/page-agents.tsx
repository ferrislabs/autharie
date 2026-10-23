import { SectionPage } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { describeAgents } from '../../agent-state'
import type { DecisionFilters } from '../../decisions'
import type { PolicyTemplateDraft } from '../../policy-templates'
import type { AgentDecision, AgentPolicy, AgentServer, AgentSettings } from '../../types'
import { AgentsHeader } from './agents-header'
import { ConnectAgent } from './connect-agent'
import { DecisionLog } from './decision-log'
import { PoliciesList } from './policies-list'
import { ServersSection } from './servers-section'

export type AgentsTabId = 'overview' | 'access-map' | 'policies' | 'activity'

interface Props {
  defaultTab?: AgentsTabId
  accessMap?: React.ReactNode
  settings?: AgentSettings
  isLoading: boolean
  error?: string
  realms: string[] | null
  realm: string
  canManage: boolean
  isSaving: boolean
  saveError?: string
  policies: AgentPolicy[]
  policiesLoading: boolean
  policiesError?: string
  busyPolicyId: string | null
  decisions: AgentDecision[]
  decisionsLoading: boolean
  decisionsError?: string
  filters: DecisionFilters
  onFilters: (filters: DecisionFilters) => void
  onRealmChange: (realm: string) => void
  onToggle: (enabled: boolean) => void
  onSaveRealm: () => void
  onOpenOrganisationSettings: () => void
  onCreatePolicy: (initial?: PolicyTemplateDraft) => void
  onEditPolicy: (policy: AgentPolicy) => void
  onDeletePolicy: (policy: AgentPolicy) => void
  onTogglePolicy: (policy: AgentPolicy, enabled: boolean) => void
  servers?: AgentServer[]
  serversLoading?: boolean
  serversError?: string
  busyServerId?: string | null
  onAddServer?: () => void
  onEditServer?: (server: AgentServer) => void
  onRemoveServer?: (server: AgentServer) => void
  onRefreshServer?: (server: AgentServer) => void
}

const NO_SERVERS: AgentServer[] = []
const noop = () => {}

export function PageAgents(props: Props) {
  if (props.isLoading) return <Skeleton className='h-64 w-full' />

  const state = props.settings ? describeAgents(props.settings, props.canManage) : null

  return (
    <SectionPage
      title='Agents'
      description='Let AI agents work with this deployment over MCP, and decide what each one may do.'
    >
      {props.error && (
        <p role='alert' className='text-sm text-destructive'>
          {props.error}
        </p>
      )}

      {props.settings && state && (
        <Tabs defaultValue={props.defaultTab ?? 'overview'} className='gap-4'>
          <TabsList>
            <TabsTrigger value='overview'>Overview</TabsTrigger>
            <TabsTrigger value='access-map'>Access map</TabsTrigger>
            <TabsTrigger value='policies'>Policies</TabsTrigger>
            <TabsTrigger value='activity'>Activity</TabsTrigger>
          </TabsList>

          <TabsContent value='overview' className='space-y-6'>
            <AgentsHeader
              settings={props.settings}
              state={state}
              realms={props.realms}
              realm={props.realm}
              canManage={props.canManage}
              isSaving={props.isSaving}
              error={props.saveError}
              onRealmChange={props.onRealmChange}
              onToggle={props.onToggle}
              onSaveRealm={props.onSaveRealm}
              onOpenOrganisationSettings={props.onOpenOrganisationSettings}
            />
            <ServersSection
              servers={props.servers ?? NO_SERVERS}
              isLoading={props.serversLoading ?? false}
              error={props.serversError}
              canManage={props.canManage}
              busyId={props.busyServerId ?? null}
              onAdd={props.onAddServer ?? noop}
              onEdit={props.onEditServer ?? noop}
              onRemove={props.onRemoveServer ?? noop}
              onRefresh={props.onRefreshServer ?? noop}
            />
            <ConnectAgent settings={props.settings} state={state} servers={props.servers} />
          </TabsContent>

          <TabsContent value='access-map'>
            {props.accessMap ?? (
              <p className='text-sm text-muted-foreground'>
                The access map shows which tools each agent can reach. It is not available yet.
              </p>
            )}
          </TabsContent>

          <TabsContent value='policies'>
            <PoliciesList
              policies={props.policies}
              isLoading={props.policiesLoading}
              error={props.policiesError}
              canManage={props.canManage}
              busyId={props.busyPolicyId}
              onCreate={props.onCreatePolicy}
              onEdit={props.onEditPolicy}
              onDelete={props.onDeletePolicy}
              onToggle={props.onTogglePolicy}
            />
          </TabsContent>

          <TabsContent value='activity'>
            <DecisionLog
              decisions={props.decisions}
              policies={props.policies}
              isLoading={props.decisionsLoading}
              error={props.decisionsError}
              filters={props.filters}
              onFilters={props.onFilters}
            />
          </TabsContent>
        </Tabs>
      )}
    </SectionPage>
  )
}
