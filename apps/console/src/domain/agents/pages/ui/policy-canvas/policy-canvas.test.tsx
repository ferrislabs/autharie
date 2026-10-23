import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { pendingChanges, docFromPolicies, newRuleBox, addBox, type RuleBox, type WhoBox, type WhatBox } from '../../../canvas-model'
import { compileRule } from '../../../compile-rule'
import { BUILTIN_CATALOGUE, catalogueOf } from '../../../tool-catalogue'
import type { AgentPolicy, RuleModel } from '../../../types'
import { CatalogueContext } from '../catalogue-context'
import { CanvasContextMenu } from './canvas-context-menu'
import { DraftBar } from './draft-bar'
import { RuleEditor } from './rule-editor'
import { PolicyCanvas } from './policy-canvas'
import { paneMenu } from './menu-items'
import { serverOptions } from './server-options'
import { ToolPicker } from './tool-picker'
import { WhatEditor } from './what-editor'
import { WhoEditor } from './who-editor'

const noop = () => {}

const model: RuleModel = {
  version: 1,
  effect: 'permit',
  who: { kind: 'roles', roles: ['admin'] },
  agents: { kind: 'any' },
  what: { kind: 'read-only', server: null },
  when: null,
}

function policy(id: string, rule?: RuleModel): AgentPolicy {
  const compiled = rule ? compileRule(rule) : null
  return {
    id,
    name: `Policy ${id}`,
    enabled: true,
    cedar: compiled && compiled.ok ? compiled.cedar : 'permit (principal, action, resource);',
    created_at: '',
    updated_at: '',
    ...(rule ? { rule } : {}),
  }
}

const publisher = {
  create: () => Promise.resolve({ id: 'x' }),
  update: () => Promise.resolve(null),
  remove: () => Promise.resolve(null),
}

describe('PolicyCanvas', () => {
  it('shows the empty hint and the template button', () => {
    const html = renderToStaticMarkup(
      <PolicyCanvas policies={[]} canManage publisher={publisher} onOpenAdvanced={noop} />,
    )
    expect(html).toContain('Right-click anywhere to add your first rule')
    expect(html).toContain('Start from a template')
    expect(html).toContain('Everything is published.')
    expect(html).toContain('Right-click to add, drag between boxes to link, click a box to edit.')
  })

  it('has no draft bar and no editing hints when read-only', () => {
    const html = renderToStaticMarkup(
      <PolicyCanvas policies={[policy('a', model)]} canManage={false} publisher={publisher} onOpenAdvanced={noop} />,
    )
    expect(html).not.toContain('Publish')
    expect(html).not.toContain('Right-click to add')
  })

  it('lists raw Cedar policies under the canvas', () => {
    const html = renderToStaticMarkup(
      <PolicyCanvas policies={[policy('a', model), policy('raw')]} canManage publisher={publisher} onOpenAdvanced={noop} />,
    )
    expect(html).toContain('Custom Cedar policies')
    expect(html).toContain('Edited in the advanced editor')
    expect(html).toContain('Policy raw')
  })
})

describe('DraftBar', () => {
  const unchanged = pendingChanges([], docFromPolicies([]))

  it('counts the unpublished changes and lists what blocks publishing', () => {
    const doc = addBox({ boxes: [], links: [], positions: {} }, newRuleBox({ boxes: [], links: [], positions: {} }))
    const pending = pendingChanges([], doc)
    const html = renderToStaticMarkup(
      <DraftBar pending={pending} progress={null} error={null} serverChanged={false} onDiscard={noop} onPublish={noop} onFocusRule={noop} />,
    )
    expect(html).toContain('1 unpublished change')
    expect(html).toContain('Fix these rules before publishing')
    expect(html).toContain('New rule')
    expect(/<button[^>]*disabled=""[^>]*>[^<]*<svg[^>]*>.*?<\/svg>Publish/.test(html)).toBe(true)
  })

  it('shows progress and errors', () => {
    const html = renderToStaticMarkup(
      <DraftBar pending={unchanged} progress={{ done: 1, total: 3 }} error='Boom' serverChanged onDiscard={noop} onPublish={noop} onFocusRule={noop} />,
    )
    expect(html).toContain('Publishing 2 of 3')
    expect(html).toContain('Boom')
    expect(html).toContain('changed while you were editing')
  })
})

describe('CanvasContextMenu', () => {
  const actions = { newRule: noop, addWho: noop, addWhat: noop, fromTemplate: noop }

  it('is a menu with the add entries', () => {
    const html = renderToStaticMarkup(
      <CanvasContextMenu x={10} y={10} width={800} height={500} items={paneMenu(actions, 'all', BUILTIN_CATALOGUE)} onClose={noop} />,
    )
    expect(html).toContain('role="menu"')
    for (const label of ['New rule', 'Add who', 'Add what', 'From template']) expect(html).toContain(label)
    expect(html).toContain('aria-haspopup="menu"')
  })

  it('restricts to the kind being linked', () => {
    const rule = renderToStaticMarkup(
      <CanvasContextMenu x={0} y={0} width={800} height={500} items={paneMenu(actions, 'rule', BUILTIN_CATALOGUE)} onClose={noop} />,
    )
    expect(rule).toContain('New rule')
    expect(rule).not.toContain('Add who')
    const what = renderToStaticMarkup(
      <CanvasContextMenu x={0} y={0} width={800} height={500} items={paneMenu(actions, 'what', BUILTIN_CATALOGUE)} onClose={noop} />,
    )
    expect(what).toContain('Read-only tools')
    expect(what).toContain('Read-only on FerrisKey')
    expect(what).toContain('Whole server')
    expect(what).toContain('Pick tools…')
    expect(what).not.toContain('New rule')
  })
})

describe('editors', () => {
  const rule: RuleBox = {
    id: 'rule:a',
    kind: 'rule',
    policyId: 'a',
    name: 'Admins read',
    description: '',
    enabled: true,
    effect: 'permit',
    agents: { kind: 'clients', clientIds: ['cursor'] },
    when: { weekdays: [1, 2], hours: { from: 9, to: 18 } },
  }

  it('lets the rule be edited, with Cedar and the advanced link', () => {
    const html = renderToStaticMarkup(
      <RuleEditor box={rule} issues={['Pick a thing.']} cedar='permit (x);' readOnly={false} canOpenAdvanced onChange={noop} onOpenAdvanced={noop} onClose={noop} />,
    )
    for (const text of ['Admins read', 'Allow', 'Deny', 'Any agent', 'Mon', 'Sun', 'Between certain hours', 'Pick a thing.', 'Cedar', 'Open in advanced editor']) {
      expect(html).toContain(text)
    }
  })

  it('hides the advanced link for unpublished rules and locks inputs when read-only', () => {
    const html = renderToStaticMarkup(
      <RuleEditor box={{ ...rule, policyId: null }} issues={[]} cedar={null} readOnly canOpenAdvanced={false} onChange={noop} onOpenAdvanced={noop} onClose={noop} />,
    )
    expect(html).not.toContain('Open in advanced editor')
    expect(html).toContain('Not published yet')
    expect(/<input[^>]*id="rule-name"[^>]*disabled=""/.test(html)).toBe(true)
  })

  it('edits who and what boxes', () => {
    const who: WhoBox = { id: 'who:1', kind: 'who', subject: { type: 'role', value: 'admin' } }
    const whoHtml = renderToStaticMarkup(<WhoEditor box={who} usage={2} readOnly={false} onChange={noop} onClose={noop} />)
    expect(whoHtml).toContain('role="combobox"')
    expect(whoHtml).toContain('value="admin"')
    expect(whoHtml).toContain('linked to 2 rules')

    const what: WhatBox = { id: 'what:1', kind: 'what', target: { type: 'tools', tools: ['ferriskey/disable_user'] } }
    const whatHtml = renderToStaticMarkup(<WhatEditor box={what} usage={1} readOnly={false} onChange={noop} onClose={noop} />)
    expect(whatHtml).toContain('Search tools')
    expect(whatHtml).toContain('destructive')
    expect(whatHtml).toContain('read-only')
    expect(whatHtml).toContain('1 tool selected')
  })
})

describe('customer servers on the canvas', () => {
  const catalogue = catalogueOf([
    {
      slug: 'crm',
      name: 'CRM',
      discovery: {
        tools: [
          { name: 'list_invoices', description: 'lists', read_only: true, destructive: false },
          { name: 'delete_invoice', read_only: false, destructive: true },
        ],
      },
    },
  ])
  const actions = { newRule: noop, addWho: noop, addWhat: noop, fromTemplate: noop }

  it('offers the whole server and read-only on it in the add menu', () => {
    const html = renderToStaticMarkup(
      <CanvasContextMenu x={0} y={0} width={800} height={500} items={paneMenu(actions, 'what', catalogue)} onClose={noop} />,
    )

    expect(html).toContain('Read-only on CRM')
    expect(html).toContain('Read-only on FerrisKey')
  })

  it('lists the servers in the submenu of whole servers', () => {
    const items = paneMenu(actions, 'what', catalogue)
    const whole = items.find((item) => item.id === 'what:server')

    expect(whole?.items?.map((item) => item.label)).toEqual(['FerrisKey', 'Autharie', 'CRM'])
  })

  it('lists the discovered tools in the picker with their hints', () => {
    const html = renderToStaticMarkup(
      <CatalogueContext.Provider value={catalogue}>
        <ToolPicker tools={['crm/delete_invoice']} onChange={noop} />
      </CatalogueContext.Provider>,
    )

    expect(html).toContain('CRM')
    expect(html).toContain('list_invoices')
    expect(html).toContain('delete_invoice')
    expect(html).toContain('read-only')
    expect(html).toContain('destructive')
  })

  it('keeps a selected tool of a server that is gone, under Other', () => {
    const html = renderToStaticMarkup(
      <CatalogueContext.Provider value={catalogue}>
        <ToolPicker tools={['erp/old_tool']} onChange={noop} />
      </CatalogueContext.Provider>,
    )

    expect(html).toContain('Other')
    expect(html).toContain('old_tool')
  })

  it('offers the customer servers where a server is picked', () => {
    expect(serverOptions(catalogue, 'crm')).toEqual([
      { value: 'ferriskey', label: 'FerrisKey' },
      { value: 'autharie', label: 'Autharie' },
      { value: 'crm', label: 'CRM' },
    ])
  })

  it('keeps a server nobody registered any more in the list, named as unknown', () => {
    expect(serverOptions(catalogue, 'erp')[0]).toEqual({
      value: 'erp',
      label: 'erp (unknown server)',
    })
    expect(serverOptions(BUILTIN_CATALOGUE, 'erp')).toHaveLength(2)
    expect(serverOptions(catalogue, null)).toHaveLength(3)
  })

  it('still offers only the built-in servers without a catalogue', () => {
    const html = renderToStaticMarkup(
      <CanvasContextMenu x={0} y={0} width={800} height={500} items={paneMenu(actions, 'what', BUILTIN_CATALOGUE)} onClose={noop} />,
    )

    expect(html).not.toContain('CRM')
  })
})
