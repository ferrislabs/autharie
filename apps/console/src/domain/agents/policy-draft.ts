import { compileRule } from './compile-rule'
import { emptyForm, formToRule, ruleToForm, type RuleForm } from './rule-form'
import type { AgentPolicy, AgentPolicyInput } from './types'

export interface PolicyDraft {
  name: string
  description: string
  enabled: boolean
  mode: 'builder' | 'cedar'
  form: RuleForm
  cedar: string
}

export function draftFromPolicy(policy: AgentPolicy | null): PolicyDraft {
  if (policy === null) {
    return { name: '', description: '', enabled: true, mode: 'builder', form: emptyForm(), cedar: '' }
  }

  return {
    name: policy.name,
    description: policy.description ?? '',
    enabled: policy.enabled,
    mode: policy.rule ? 'builder' : 'cedar',
    form: policy.rule ? ruleToForm(policy.rule) : emptyForm(),
    cedar: policy.cedar,
  }
}

export function cedarOf(draft: PolicyDraft): { cedar: string; issues: string[] } {
  if (draft.mode === 'cedar') {
    return draft.cedar.trim() === ''
      ? { cedar: '', issues: ['Write the policy.'] }
      : { cedar: draft.cedar, issues: [] }
  }

  const compiled = compileRule(formToRule(draft.form))
  return compiled.ok ? { cedar: compiled.cedar, issues: [] } : { cedar: '', issues: compiled.issues }
}

export function switchToCedar(draft: PolicyDraft): PolicyDraft {
  if (draft.mode === 'cedar') return draft
  const { cedar, issues } = cedarOf(draft)
  if (issues.length > 0) return draft
  return { ...draft, mode: 'cedar', cedar }
}

export function nameIssue(name: string): string | null {
  const trimmed = name.trim()
  if (trimmed === '') return 'Give the policy a name.'
  if (trimmed.length > 80) return 'That is longer than 80 characters.'
  return null
}

export function toInput(draft: PolicyDraft): AgentPolicyInput | null {
  if (nameIssue(draft.name) !== null) return null
  const { cedar, issues } = cedarOf(draft)
  if (issues.length > 0) return null

  const input: AgentPolicyInput = {
    name: draft.name.trim(),
    enabled: draft.enabled,
    cedar,
  }
  const description = draft.description.trim()
  if (description !== '') input.description = description
  if (draft.mode === 'builder') input.rule = formToRule(draft.form)
  return input
}
