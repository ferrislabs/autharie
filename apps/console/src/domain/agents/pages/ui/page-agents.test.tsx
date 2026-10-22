import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { emptyFilters } from '../../decisions'
import type { AgentDecision, AgentPolicy, AgentSettings } from '../../types'
import { PageAgents } from './page-agents'

const noop = () => {}

function switchDisabled(html: string): boolean {
  const tag = /<button[^>]*role="switch"[^>]*id="agents-enabled"[^>]*>/.exec(html)
  if (!tag) throw new Error('no switch')
  return tag[0].includes('disabled=""')
}

function render(over: Partial<React.ComponentProps<typeof PageAgents>> = {}) {
  return renderToStaticMarkup(
    <PageAgents
      isLoading={false}
      settings={{ enabled: false, realm: '', organisation_mcp_enabled: true }}
      realms={null}
      realm=''
      canManage
      isSaving={false}
      policies={[]}
      policiesLoading={false}
      busyPolicyId={null}
      decisions={[]}
      decisionsLoading={false}
      filters={emptyFilters()}
      onFilters={noop}
      onRealmChange={noop}
      onToggle={noop}
      onSaveRealm={noop}
      onOpenOrganisationSettings={noop}
      onCreatePolicy={noop}
      onEditPolicy={noop}
      onDeletePolicy={noop}
      onTogglePolicy={noop}
      {...over}
    />,
  )
}

function withSettings(settings: Partial<AgentSettings>) {
  return render({
    realm: settings.realm ?? '',
    settings: { enabled: false, realm: '', organisation_mcp_enabled: true, ...settings },
  })
}

const policy: AgentPolicy = {
  id: 'p1',
  name: 'Support reads',
  enabled: true,
  phase: 'Failed',
  message: 'rejected by the realm',
  cedar: 'permit (principal, action, resource);',
  created_at: '',
  updated_at: '',
}

const decision: AgentDecision = {
  kind: 'agent_decision',
  time: '2026-10-22T10:00:00Z',
  server: 'autharie',
  method: 'tools/call',
  tool: 'autharie/list_backups',
  subject_type: 'User',
  subject_id: 'u-1',
  agent: 'claude',
  delegated: true,
  decision: 'deny',
  policies: ['policy-9'],
  reason: 'no permit',
  request_id: 'r1',
  deployment_id: 'd1',
}

describe('the agents tab states', () => {
  it('explains the organisation switch is off and locks the agents switch', () => {
    const html = withSettings({ organisation_mcp_enabled: false })
    expect(html).toContain('AI access is off for this organisation')
    expect(html).toContain('Open organisation settings')
    expect(switchDisabled(html)).toBe(true)
  })

  it('shows a disabled deployment with a free switch and no callout', () => {
    const html = withSettings({ realm: 'master' })
    expect(html).not.toContain('AI access is off')
    expect(switchDisabled(html)).toBe(false)
    expect(html).toContain('Off<span')
  })

  it('keeps the switch off until a realm is chosen', () => {
    expect(switchDisabled(withSettings({}))).toBe(true)
  })

  it('shows a ready deployment with its MCP URL and gateway client', () => {
    const html = withSettings({
      enabled: true,
      realm: 'master',
      mcp_url: 'https://dep.example.com/mcp',
      status: { phase: 'Ready', gateway_client_ready: true, updated_at: '' },
    })
    expect(html).toContain('https://dep.example.com/mcp')
    expect(html).toContain('Copy the MCP URL')
    expect(html).toContain('Ready')
  })

  it('shows the failure message', () => {
    const html = withSettings({
      enabled: true,
      realm: 'master',
      status: { phase: 'Failed', gateway_client_ready: false, message: 'realm missing', updated_at: '' },
    })
    expect(html).toContain('realm missing')
  })

  it('falls back to a text field for the realm and uses a picker when realms are known', () => {
    expect(render()).toContain('placeholder="master"')
    expect(render({ realms: ['master', 'acme'] })).not.toContain('placeholder="master"')
  })

  it('locks the switch for somebody who cannot manage', () => {
    expect(switchDisabled(render({ canManage: false }))).toBe(true)
  })
})

describe('the policies list', () => {
  it('teaches default deny and offers the first policy', () => {
    const html = render()
    expect(html).toContain('No policies yet')
    expect(html).toContain('Create your first policy')
  })

  it('shows name, effect summary and phase with its message', () => {
    const html = render({ policies: [policy] })
    expect(html).toContain('Support reads')
    expect(html).toContain('Permit · custom Cedar')
    expect(html).toContain('Failed')
    expect(html).toContain('rejected by the realm')
    expect(html).toContain('Edit Support reads')
    expect(html).toContain('Delete Support reads')
  })

  it('hides editing from a member who cannot manage', () => {
    const html = render({ policies: [policy], canManage: false })
    expect(html).not.toContain('New policy')
    expect(html).not.toContain('Edit Support reads')
  })

  it('says a disabled policy is not applied', () => {
    expect(render({ policies: [{ ...policy, enabled: false }] })).toContain('Not applied')
  })
})

describe('the decision log', () => {
  it('shows the empty state', () => {
    expect(render()).toContain('No decisions yet')
  })

  it('says when filters hide everything', () => {
    expect(render({ filters: { ...emptyFilters(), decision: 'deny' } })).toContain(
      'No decision matches these filters',
    )
    expect(render({ filters: { ...emptyFilters(), decision: 'deny' } })).toContain('Clear filters')
  })

  it('shows a decision row', () => {
    const html = render({ decisions: [decision] })
    expect(html).toContain('Deny')
    expect(html).toContain('autharie/list_backups')
    expect(html).toContain('User u-1')
    expect(html).toContain('claude')
    expect(html).toContain('policy-9')
    expect(html).toContain('no permit')
    expect(html).toContain('2026-10-22 10:00:00')
  })
})
