import { describe, expect, it } from 'vitest'
import { compileRule } from './compile-rule'
import {
  cedarOf,
  draftFromPolicy,
  nameIssue,
  switchToCedar,
  toInput,
  type PolicyDraft,
} from './policy-draft'
import { emptyForm, formToRule } from './rule-form'
import type { AgentPolicy } from './types'

const CEDAR = 'permit (principal, action, resource);'

function stored(over: Partial<AgentPolicy> = {}): AgentPolicy {
  return {
    id: 'p1',
    name: 'Support reads',
    description: 'desc',
    enabled: false,
    cedar: CEDAR,
    created_at: '',
    updated_at: '',
    ...over,
  }
}

function draft(over: Partial<PolicyDraft> = {}): PolicyDraft {
  return { ...draftFromPolicy(null), name: 'Support reads', ...over }
}

describe('starting a draft', () => {
  it('starts a new policy in the builder, enabled, with the empty form', () => {
    const fresh = draftFromPolicy(null)
    expect(fresh).toMatchObject({ name: '', description: '', enabled: true, mode: 'builder' })
    expect(fresh.form).toEqual(emptyForm())
  })

  it('opens a builder policy in the builder with its rule', () => {
    const rule = formToRule({ ...emptyForm(), who: 'groups', groups: 'support' })
    const opened = draftFromPolicy(stored({ rule }))
    expect(opened.mode).toBe('builder')
    expect(formToRule(opened.form)).toEqual(rule)
    expect(opened.enabled).toBe(false)
  })

  it('opens a policy without a rule in Cedar mode with its text', () => {
    const opened = draftFromPolicy(stored())
    expect(opened.mode).toBe('cedar')
    expect(opened.cedar).toBe(CEDAR)
  })
})

describe('the Cedar of a draft', () => {
  it('compiles the form in builder mode', () => {
    const result = cedarOf(draft())
    const compiled = compileRule(formToRule(emptyForm()))
    expect(result.cedar).toBe(compiled.ok ? compiled.cedar : '')
    expect(result.issues).toEqual([])
  })

  it('reports the rule’s issues instead of Cedar', () => {
    const result = cedarOf(draft({ form: { ...emptyForm(), who: 'groups' } }))
    expect(result.cedar).toBe('')
    expect(result.issues.length).toBeGreaterThan(0)
  })

  it('returns the typed text in Cedar mode', () => {
    expect(cedarOf(draft({ mode: 'cedar', cedar: CEDAR }))).toEqual({ cedar: CEDAR, issues: [] })
  })

  it('refuses blank Cedar', () => {
    expect(cedarOf(draft({ mode: 'cedar', cedar: '  ' })).issues).toEqual(['Write the policy.'])
  })
})

describe('switching to Cedar', () => {
  it('carries the compiled text over and leaves the builder', () => {
    const switched = switchToCedar(draft())
    expect(switched.mode).toBe('cedar')
    expect(switched.cedar).toBe(cedarOf(draft()).cedar)
  })

  it('does nothing when the rule does not compile', () => {
    const broken = draft({ form: { ...emptyForm(), who: 'roles' } })
    expect(switchToCedar(broken)).toBe(broken)
  })

  it('leaves a Cedar draft as it is', () => {
    const cedar = draft({ mode: 'cedar', cedar: CEDAR })
    expect(switchToCedar(cedar)).toBe(cedar)
  })
})

describe('what is saved', () => {
  it('sends the rule and its Cedar from the builder', () => {
    const input = toInput(draft())
    expect(input?.rule).toEqual(formToRule(emptyForm()))
    expect(input?.cedar).toBe(cedarOf(draft()).cedar)
    expect(input).toMatchObject({ name: 'Support reads', enabled: true })
  })

  it('sends no rule from Cedar mode', () => {
    const input = toInput(draft({ mode: 'cedar', cedar: CEDAR }))
    expect(input).not.toHaveProperty('rule')
    expect(input?.cedar).toBe(CEDAR)
  })

  it('trims the name and leaves a blank description out', () => {
    const input = toInput(draft({ name: '  Support reads ', description: '  ' }))
    expect(input?.name).toBe('Support reads')
    expect(input).not.toHaveProperty('description')
  })

  it('keeps a description', () => {
    expect(toInput(draft({ description: ' for support ' }))?.description).toBe('for support')
  })

  it('refuses a blank name', () => {
    expect(toInput(draft({ name: ' ' }))).toBeNull()
  })

  it('refuses a rule that does not compile', () => {
    expect(toInput(draft({ form: { ...emptyForm(), who: 'groups' } }))).toBeNull()
  })
})

describe('the name', () => {
  it('needs a name', () => {
    expect(nameIssue('  ')).toBe('Give the policy a name.')
  })

  it('caps the length', () => {
    expect(nameIssue('x'.repeat(81))).toBe('That is longer than 80 characters.')
  })

  it('accepts a name', () => {
    expect(nameIssue('Support reads')).toBeNull()
  })
})
