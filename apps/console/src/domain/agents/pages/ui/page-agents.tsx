import { SectionPage } from '@/components/layout/page'
import { Skeleton } from '@/components/ui/skeleton'
import { describeAgents } from '../../agent-state'
import type { DecisionFilters } from '../../decisions'
import type { AgentDecision, AgentPolicy, AgentSettings } from '../../types'
import { AgentsHeader } from './agents-header'
import { DecisionLog } from './decision-log'
import { PoliciesList } from './policies-list'

interface Props {
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
  onCreatePolicy: () => void
  onEditPolicy: (policy: AgentPolicy) => void
  onDeletePolicy: (policy: AgentPolicy) => void
  onTogglePolicy: (policy: AgentPolicy, enabled: boolean) => void
}

export function PageAgents(props: Props) {
  if (props.isLoading) return <Skeleton className='h-64 w-full' />

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

      {props.settings && (
        <AgentsHeader
          settings={props.settings}
          state={describeAgents(props.settings, props.canManage)}
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
      )}

      {props.settings && (
        <>
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

          <DecisionLog
            decisions={props.decisions}
            policies={props.policies}
            isLoading={props.decisionsLoading}
            error={props.decisionsError}
            filters={props.filters}
            onFilters={props.onFilters}
          />
        </>
      )}
    </SectionPage>
  )
}
