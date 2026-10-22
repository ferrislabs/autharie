import type * as CedarModule from '@cedar-policy/cedar-wasm/web'
import { AGENT_SCHEMA } from './cedar-schema'
import { findTool, nameOfTool, serverOfTool } from './tool-catalogue'

export type CedarApi = Pick<typeof CedarModule, 'checkParsePolicySet' | 'validate' | 'isAuthorized'>

export interface Issue {
  message: string
  help?: string
}

export interface Validation {
  syntax: Issue[]
  errors: Issue[]
  warnings: Issue[]
}

export type Subject =
  | { kind: 'user'; id: string; groups: string[]; roles: string[]; agent: string }
  | { kind: 'agent'; clientId: string }

export interface Scenario {
  subject: Subject
  tool: string
  time: Date
}

export interface NamedPolicy {
  id: string
  name: string
  cedar: string
}

export interface Outcome {
  decision: 'allow' | 'deny'
  determining: NamedPolicy[]
  errors: Issue[]
}

function issueOf(error: { message: string; help: string | null }): Issue {
  return error.help ? { message: error.message, help: error.help } : { message: error.message }
}

export function validatePolicy(api: CedarApi, cedar: string): Validation {
  const parsed = api.checkParsePolicySet({ staticPolicies: cedar })
  if (parsed.type === 'failure') {
    return { syntax: parsed.errors.map(issueOf), errors: [], warnings: [] }
  }

  const answer = api.validate({ schema: AGENT_SCHEMA, policies: { staticPolicies: cedar } })
  if (answer.type === 'failure') {
    return { syntax: [], errors: answer.errors.map(issueOf), warnings: answer.warnings.map(issueOf) }
  }

  return {
    syntax: [],
    errors: answer.validationErrors.map((entry) => issueOf(entry.error)),
    warnings: [...answer.validationWarnings, ...answer.otherWarnings.map((error) => ({ error }))].map(
      (entry) => issueOf(entry.error),
    ),
  }
}

export function isValid(validation: Validation): boolean {
  return validation.syntax.length === 0 && validation.errors.length === 0
}

export function contextTime(at: Date): { weekday: number; hour: number } {
  const day = at.getUTCDay()
  return { weekday: day === 0 ? 7 : day, hour: at.getUTCHours() }
}

function uid(type: string, id: string) {
  return { type: `FerrisKey::${type}`, id }
}

function entities(scenario: Scenario) {
  const server = serverOfTool(scenario.tool)
  const list: unknown[] = [
    { uid: uid('McpServer', server), attrs: {}, parents: [] },
    {
      uid: uid('Tool', scenario.tool),
      attrs: {
        server,
        tool: nameOfTool(scenario.tool),
        annotations: toolAnnotations(scenario.tool),
      },
      parents: [uid('McpServer', server)],
    },
  ]

  if (scenario.subject.kind === 'user') {
    const groups = [...new Set(scenario.subject.groups)]
    for (const group of groups) list.push({ uid: uid('Group', group), attrs: {}, parents: [] })
    list.push({
      uid: uid('User', scenario.subject.id),
      attrs: { roles: [...new Set(scenario.subject.roles)], groups },
      parents: groups.map((group) => uid('Group', group)),
    })
  } else {
    list.push({ uid: uid('Agent', scenario.subject.clientId), attrs: {}, parents: [] })
  }

  return list
}

function toolAnnotations(toolId: string) {
  const tool = findTool(toolId)
  return { readOnlyHint: tool?.readOnly ?? false, destructiveHint: tool?.destructive ?? false }
}

export function simulate(api: CedarApi, policies: NamedPolicy[], scenario: Scenario): Outcome {
  const set: Record<string, string> = {}
  for (const policy of policies) set[policy.id] = policy.cedar

  const subject = scenario.subject
  const principal =
    subject.kind === 'user' ? uid('User', subject.id) : uid('Agent', subject.clientId)

  const answer = api.isAuthorized({
    principal,
    action: uid('Action', 'call_tool'),
    resource: uid('Tool', scenario.tool),
    context: {
      agent: subject.kind === 'user' ? subject.agent : subject.clientId,
      delegated: subject.kind === 'user',
      arguments: {},
      time: contextTime(scenario.time),
    },
    schema: AGENT_SCHEMA,
    validateRequest: true,
    policies: { staticPolicies: set },
    entities: entities(scenario) as never,
  })

  if (answer.type === 'failure') {
    return { decision: 'deny', determining: [], errors: answer.errors.map(issueOf) }
  }

  const byId = new Map(policies.map((policy) => [policy.id, policy]))
  const determining = answer.response.diagnostics.reason
    .map((id) => byId.get(id))
    .filter((policy): policy is NamedPolicy => policy !== undefined)

  return {
    decision: answer.response.decision,
    determining,
    errors: answer.response.diagnostics.errors.map((error) => ({
      message: `${byId.get(error.policyId)?.name ?? error.policyId}: ${error.error.message}`,
    })),
  }
}

let loading: Promise<CedarApi> | null = null

export function loadCedar(): Promise<CedarApi> {
  loading ??= import('@cedar-policy/cedar-wasm/web').then(async (module) => {
    await module.default()
    return module
  })
  loading.catch(() => {
    loading = null
  })
  return loading
}
