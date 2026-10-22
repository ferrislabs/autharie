import { fetcher } from '@/api/api.fetch'
import type {
  AgentDecision,
  AgentPolicy,
  AgentPolicyInput,
  AgentSettings,
  AgentSettingsInput,
  DecisionQuery,
  Scope,
} from '../types'

type Verb = 'get' | 'put' | 'post' | 'delete'

interface Params {
  body?: unknown
  query?: Record<string, unknown>
}

function root(scope: Scope): string {
  const organisation = encodeURIComponent(scope.organisationId ?? '')
  const deployment = encodeURIComponent(scope.deploymentId ?? '')
  return `${window.apiUrl}/organisations/${organisation}/deployments/${deployment}`
}

async function call<T>(verb: Verb, scope: Scope, path: string, params: Params = {}): Promise<T> {
  const response = await fetcher(verb, `${root(scope)}${path}`, params)
  if (response.status === 204) return undefined as T
  const text = await response.text()
  return (text === '' ? undefined : JSON.parse(text)) as T
}

async function data<T>(verb: Verb, scope: Scope, path: string, params: Params = {}): Promise<T> {
  const body = await call<{ data: T }>(verb, scope, path, params)
  return body.data
}

export function getSettings(scope: Scope): Promise<AgentSettings> {
  return data('get', scope, '/agents')
}

export function updateSettings(scope: Scope, input: AgentSettingsInput): Promise<AgentSettings> {
  return data('put', scope, '/agents', { body: input })
}

export function listPolicies(scope: Scope): Promise<AgentPolicy[]> {
  return data('get', scope, '/agents/policies')
}

export function createPolicy(scope: Scope, input: AgentPolicyInput): Promise<AgentPolicy> {
  return data('post', scope, '/agents/policies', { body: input })
}

export function updatePolicy(
  scope: Scope,
  policyId: string,
  input: AgentPolicyInput,
): Promise<AgentPolicy> {
  return data('put', scope, `/agents/policies/${encodeURIComponent(policyId)}`, { body: input })
}

export async function deletePolicy(scope: Scope, policyId: string): Promise<void> {
  await call('delete', scope, `/agents/policies/${encodeURIComponent(policyId)}`)
}

export function listDecisions(scope: Scope, query: DecisionQuery): Promise<AgentDecision[]> {
  return data('get', scope, '/agents/decisions', { query: { ...query } })
}

export function listRealms(scope: Scope): Promise<string[]> {
  return data('get', scope, '/webhook-realms')
}
