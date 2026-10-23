import { describe, expect, it } from 'vitest'
import { compileRule } from './compile-rule'
import { draftFromTemplate, toInput } from './policy-draft'
import { POLICY_TEMPLATES } from './policy-templates'
import { BUILTIN_CATALOGUE, catalogueOf, toolId } from './tool-catalogue'

function built(id: string) {
  const template = POLICY_TEMPLATES.find((entry) => entry.id === id)
  if (!template) throw new Error('no template')
  return template.build(BUILTIN_CATALOGUE)
}

describe('POLICY_TEMPLATES', () => {
  it('has unique ids and copy', () => {
    expect(new Set(POLICY_TEMPLATES.map((entry) => entry.id)).size).toBe(POLICY_TEMPLATES.length)
    for (const template of POLICY_TEMPLATES) {
      expect(template.title).not.toBe('')
      expect(template.description).not.toBe('')
    }
  })

  it('builds rules that compile and names that fit', () => {
    for (const template of POLICY_TEMPLATES) {
      const draft = template.build(BUILTIN_CATALOGUE)
      expect(compileRule(draft.rule).ok).toBe(true)
      expect(draft.name.length).toBeLessThanOrEqual(80)
    }
  })

  it('opens in the builder as a saveable policy with the same rule', () => {
    for (const template of POLICY_TEMPLATES) {
      const initial = template.build(BUILTIN_CATALOGUE)
      const draft = draftFromTemplate(initial)
      expect(draft.mode).toBe('builder')
      expect(draft.name).toBe(initial.name)
      const input = toInput(draft)
      expect(input).not.toBeNull()
      expect(input?.rule).toEqual(initial.rule)
    }
  })

  it('builds a fresh rule each time', () => {
    const first = POLICY_TEMPLATES[0].build(BUILTIN_CATALOGUE)
    const second = POLICY_TEMPLATES[0].build(BUILTIN_CATALOGUE)
    expect(first.rule).not.toBe(second.rule)
  })

  it('forbids exactly the destructive tools', () => {
    const rule = built('block-destructive').rule
    expect(rule.effect).toBe('forbid')
    if (rule.what.kind !== 'tools') throw new Error('expected tools')
    expect(rule.what.tools).toEqual(BUILTIN_CATALOGUE.tools.filter((tool) => tool.destructive).map(toolId))
    expect(rule.what.tools.length).toBeGreaterThan(0)
  })

  it('forbids the destructive tools that customer servers announced too', () => {
    const catalogue = catalogueOf([
      {
        slug: 'crm',
        name: 'CRM',
        discovery: {
          tools: [
            { name: 'list_invoices', read_only: true, destructive: false },
            { name: 'delete_invoice', read_only: false, destructive: true },
          ],
        },
      },
    ])

    const draft = POLICY_TEMPLATES.find((entry) => entry.id === 'block-destructive')?.build(catalogue)

    if (draft?.rule.what.kind !== 'tools') throw new Error('expected tools')
    expect(draft.rule.what.tools).toContain('crm/delete_invoice')
    expect(draft.rule.what.tools).not.toContain('crm/list_invoices')
  })

  it('scopes admins to ferriskey', () => {
    const rule = built('admins-ferriskey').rule
    expect(rule.who).toEqual({ kind: 'roles', roles: ['admin'] })
    expect(rule.what).toEqual({ kind: 'server', server: 'ferriskey' })
  })

  it('limits business hours to Monday to Friday, 9 to 18', () => {
    const compiled = compileRule(built('read-only-business-hours').rule)
    if (!compiled.ok) throw new Error('did not compile')
    expect(compiled.cedar).toContain('[1, 2, 3, 4, 5].contains(context.time.weekday)')
    expect(compiled.cedar).toContain('context.time.hour >= 9 && context.time.hour < 18')
  })

  it('permits read-only for everyone on every server', () => {
    const rule = built('read-only').rule
    expect(rule.effect).toBe('permit')
    expect(rule.what).toEqual({ kind: 'read-only', server: null })
    expect(rule.when).toBeNull()
  })
})
