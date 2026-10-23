import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { selectAccessToken, useAuthStore } from '@/stores/auth'
import { DECISIONS_REFRESH_MS, buildDecisionQuery, type DecisionFilters } from '../decisions'
import {
  createPolicy,
  createServer,
  deletePolicy,
  deleteServer,
  discoverServer,
  getSettings,
  listDecisions,
  listPolicies,
  listRealms,
  listServers,
  updatePolicy,
  updateServer,
  updateSettings,
} from '../services/agents.service'
import type {
  AgentPolicyInput,
  AgentServer,
  AgentServerChange,
  AgentServerCreate,
  AgentSettingsInput,
  Scope,
} from '../types'

const SETTLING_REFRESH_MS = 3000

function key(scope: Scope, ...rest: unknown[]) {
  return ['agents', scope.organisationId, scope.deploymentId, ...rest]
}

function useEnabled(scope: Scope) {
  const accessToken = useAuthStore(selectAccessToken)
  return !!scope.organisationId && !!scope.deploymentId && !!accessToken
}

export function useAgentSettings(scope: Scope) {
  const enabled = useEnabled(scope)

  return useQuery({
    queryKey: key(scope, 'settings'),
    queryFn: () => getSettings(scope),
    enabled,
    refetchInterval: (query) => {
      const settings = query.state.data
      return settings?.enabled && settings.status?.phase !== 'Ready' ? SETTLING_REFRESH_MS : false
    },
  })
}

export function useUpdateAgentSettings(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (input: AgentSettingsInput) => updateSettings(scope, input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'settings') }),
  })
}

export function useAgentRealms(scope: Scope) {
  const enabled = useEnabled(scope)

  return useQuery({
    queryKey: key(scope, 'realms'),
    queryFn: () => listRealms(scope),
    enabled,
    retry: false,
    staleTime: 5 * 60 * 1000,
  })
}

export function useAgentPolicies(scope: Scope) {
  const enabled = useEnabled(scope)

  return useQuery({
    queryKey: key(scope, 'policies'),
    queryFn: () => listPolicies(scope),
    enabled,
    refetchInterval: (query) =>
      query.state.data?.some((policy) => policy.enabled && policy.phase === 'Pending')
        ? SETTLING_REFRESH_MS
        : false,
  })
}

export function useSavePolicy(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ id, input }: { id: string | null; input: AgentPolicyInput }) =>
      id === null ? createPolicy(scope, input) : updatePolicy(scope, id, input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'policies') }),
  })
}

export function useDeletePolicy(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (policyId: string) => deletePolicy(scope, policyId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'policies') }),
  })
}

export function useAgentDecisions(scope: Scope, filters: DecisionFilters) {
  const enabled = useEnabled(scope)

  return useQuery({
    queryKey: key(scope, 'decisions', filters),
    queryFn: () => listDecisions(scope, buildDecisionQuery(filters, new Date())),
    enabled,
    refetchInterval: DECISIONS_REFRESH_MS,
  })
}

export type SaveServer =
  | { kind: 'create'; input: AgentServerCreate }
  | { kind: 'change'; id: string; input: AgentServerChange }

export function needsDiscovery(servers: AgentServer[] | undefined): boolean {
  return (servers ?? []).some((server) => server.enabled && server.discovery === undefined)
}

export function useAgentServers(scope: Scope) {
  const enabled = useEnabled(scope)

  return useQuery({
    queryKey: key(scope, 'servers'),
    queryFn: () => listServers(scope),
    enabled,
    refetchInterval: (query) => (needsDiscovery(query.state.data) ? SETTLING_REFRESH_MS : false),
  })
}

export function useSaveServer(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (save: SaveServer) =>
      save.kind === 'create'
        ? createServer(scope, save.input)
        : updateServer(scope, save.id, save.input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'servers') }),
  })
}

export function useDeleteServer(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (serverId: string) => deleteServer(scope, serverId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'servers') }),
  })
}

export function useDiscoverServer(scope: Scope) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (serverId: string) => discoverServer(scope, serverId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(scope, 'servers') }),
  })
}
