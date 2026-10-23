import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { emptyFilters } from '../../decisions'
import type { AgentDecision, AgentPolicy, AgentServer, AgentSettings } from '../../types'
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

describe('the tabs', () => {
  it('lists the tabs, with Agents between the access map and the policies, and opens on the overview', () => {
    const html = render()
    for (const label of ['Overview', 'Access map', 'Policies', 'Activity']) {
      expect(html).toContain(`>${label}</button>`)
    }
    expect(html).toMatch(/Access map<\/button>.*?Agents.*?Policies<\/button>/s)
    expect(html).toContain('Agents on this deployment')
    expect(html).not.toContain('No policies yet')
    expect(html).not.toContain('No decisions yet')
  })

  it('shows a placeholder or the given access map', () => {
    expect(render({ defaultTab: 'access-map' })).toContain('not available yet')
    const html = render({ defaultTab: 'access-map', accessMap: <p>the map</p> })
    expect(html).toContain('the map')
    expect(html).not.toContain('not available yet')
  })
})

describe('the connection guide', () => {
  it('explains what is missing when agents are off', () => {
    const html = withSettings({ realm: 'master' })
    expect(html).toContain('Connect an agent')
    expect(html).toContain('Turn agents on')
    expect(html).not.toContain('claude mcp add')
  })

  it('explains the organisation switch when it is off', () => {
    const html = withSettings({ organisation_mcp_enabled: false })
    expect(html).toContain('Connect an agent')
    expect(html.split('AI access is off for this organisation').length).toBeGreaterThan(2)
  })

  it('shows the client picker and the Claude Code command when ready', () => {
    const html = withSettings({
      enabled: true,
      realm: 'master',
      mcp_url: 'https://dep.example.com/mcp',
      status: { phase: 'Ready', gateway_client_ready: true, updated_at: '' },
    })
    expect(html).toContain('Cursor')
    expect(html).toContain('VS Code')
    expect(html).toContain('claude mcp add --transport http autharie https://dep.example.com/mcp')
    expect(html).toContain('watch the Activity tab')
  })
})

describe('the policies list', () => {
  it('teaches default deny and offers the first policy', () => {
    const html = render({ defaultTab: 'policies' })
    expect(html).toContain('No policies yet')
    expect(html).toContain('Create your first policy')
  })

  it('offers the templates in the empty state', () => {
    const html = render({ defaultTab: 'policies' })
    expect(html).toContain('Read-only for everyone')
    expect(html).toContain('Block destructive tools')
  })

  it('shows name, effect summary and phase with its message', () => {
    const html = render({ defaultTab: 'policies', policies: [policy] })
    expect(html).toContain('Support reads')
    expect(html).toContain('Permit · custom Cedar')
    expect(html).toContain('Failed')
    expect(html).toContain('rejected by the realm')
    expect(html).toContain('Edit Support reads')
    expect(html).toContain('Delete Support reads')
  })

  it('hides editing from a member who cannot manage', () => {
    const html = render({ defaultTab: 'policies', policies: [policy], canManage: false })
    expect(html).not.toContain('New policy')
    expect(html).not.toContain('Edit Support reads')
  })

  it('says a disabled policy is not applied', () => {
    expect(render({ defaultTab: 'policies', policies: [{ ...policy, enabled: false }] })).toContain(
      'Not applied',
    )
  })
})

describe('the decision log', () => {
  it('shows the empty state', () => {
    expect(render({ defaultTab: 'activity' })).toContain('No decisions yet')
  })

  it('says when filters hide everything', () => {
    const filters = { ...emptyFilters(), decision: 'deny' as const }
    expect(render({ defaultTab: 'activity', filters })).toContain('No decision matches these filters')
    expect(render({ defaultTab: 'activity', filters })).toContain('Clear filters')
  })

  it('shows a decision row', () => {
    const html = render({ defaultTab: 'activity', decisions: [decision] })
    expect(html).toContain('Deny')
    expect(html).toContain('autharie/list_backups')
    expect(html).toContain('User u-1')
    expect(html).toContain('claude')
    expect(html).toContain('policy-9')
    expect(html).toContain('no permit')
    expect(html).toContain('2026-10-22 10:00:00')
  })
})

describe('the agents tab', () => {
  it('shows the given registry', () => {
    const html = render({ defaultTab: 'agents', registry: <p>the registry</p> })

    expect(html).toContain('the registry')
    expect(html).not.toContain('not available yet')
  })

  it('shows a placeholder without a registry', () => {
    expect(render({ defaultTab: 'agents' })).toContain('The agent registry is not available yet')
  })

  it('badges the tab with the number of pending agents', () => {
    const html = render({ pendingAgents: 3 })

    expect(html).toContain('aria-label="3 pending"')
  })

  it('has no badge when nothing waits for approval', () => {
    expect(render({ pendingAgents: 0 })).not.toContain('pending"')
    expect(render()).not.toContain('pending"')
  })
})

describe('the activity tab and agent approval', () => {
  const waiting: AgentDecision = {
    ...decision,
    decision: 'deny',
    reason: 'agent_pending',
    agent: 'new-agent',
    policies: [],
  }

  it('labels a refusal for a pending agent as waiting for approval, with an Approve shortcut', () => {
    const html = render({
      defaultTab: 'activity',
      decisions: [waiting],
      onApproveAgent: noop,
    })

    expect(html).toContain('Waiting for approval')
    expect(html).toContain('>Approve<')
    expect(html).not.toContain('agent_pending')
  })

  it('labels a refusal for a blocked agent without an Approve shortcut', () => {
    const html = render({
      defaultTab: 'activity',
      decisions: [{ ...waiting, reason: 'agent_blocked' }],
      onApproveAgent: noop,
    })

    expect(html).toContain('Agent blocked')
    expect(html).not.toContain('>Approve<')
  })

  it('offers no Approve shortcut to somebody who cannot approve', () => {
    const html = render({ defaultTab: 'activity', decisions: [waiting] })

    expect(html).toContain('Waiting for approval')
    expect(html).not.toContain('>Approve<')
  })

  it('filters the log by agent', () => {
    const html = render({ defaultTab: 'activity' })

    expect(html).toContain('aria-label="Agent"')
  })
})

const crm: AgentServer = {
  id: 's1',
  slug: 'crm',
  name: 'CRM',
  url: 'https://tools.example.com/mcp',
  enabled: true,
  mcp_url: 'https://dep.example.com/mcp/crm',
  discovery: {
    phase: 'Ready',
    tools: [
      { name: 'list_invoices', read_only: true, destructive: false },
      { name: 'delete_invoice', read_only: false, destructive: true },
    ],
    dropped: 0,
    discovered_at: '2026-10-23T10:00:00Z',
    protocol_version: '2025-11-25',
  },
  created_at: '',
  updated_at: '',
}

const ready: Partial<AgentSettings> = {
  enabled: true,
  realm: 'master',
  mcp_url: 'https://dep.example.com/mcp',
  status: { phase: 'Ready', gateway_client_ready: true, updated_at: '' },
}

function withServers(servers: AgentServer[], over: Partial<React.ComponentProps<typeof PageAgents>> = {}) {
  return render({
    realm: 'master',
    settings: { enabled: false, realm: 'master', organisation_mcp_enabled: true, ...ready },
    servers,
    ...over,
  })
}

describe('your MCP servers', () => {
  it('invites to register a first server', () => {
    const html = withServers([])

    expect(html).toContain('Your MCP servers')
    expect(html).toContain('No MCP server of yours yet')
    expect(html).toContain('Add a server')
  })

  it('lists each server with its address, status, tool count and last discovery', () => {
    const html = withServers([crm])

    expect(html).toContain('CRM')
    expect(html).toContain('https://tools.example.com/mcp')
    expect(html).toContain('Ready')
    expect(html).toContain('2 tools')
    expect(html).toContain('Last discovery 2026-10-23 10:00 UTC')
    expect(html).toContain('MCP 2025-11-25')
    expect(html).toContain('Refresh the tools of CRM')
    expect(html).toContain('Edit CRM')
    expect(html).toContain('Remove CRM')
  })

  it('says a server is still being discovered, or could not be reached', () => {
    expect(withServers([{ ...crm, discovery: undefined }])).toContain('Discovering tools')

    const html = withServers([
      { ...crm, discovery: { ...crm.discovery!, phase: 'Unreachable', tools: [], message: 'the server did not answer in time' } },
    ])
    expect(html).toContain('Unreachable')
    expect(html).toContain('the server did not answer in time')
  })

  it('says a server is refused when its address is not a public one', () => {
    const html = withServers([
      { ...crm, discovery: { ...crm.discovery!, phase: 'Refused', tools: [], message: 'the server’s address is not a public one' } },
    ])

    expect(html).toContain('Refused')
    expect(html).toContain('not a public one')
  })

  it('shows a switched-off server as off and cannot refresh it', () => {
    const html = withServers([{ ...crm, enabled: false }])

    expect(html).toContain('Off')
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Refresh the tools of CRM"/)
  })

  it('leaves the buttons to those who manage the deployment', () => {
    const html = withServers([crm], { canManage: false })

    expect(html).toContain('CRM')
    expect(html).not.toContain('Add a server')
    expect(html).not.toContain('Remove CRM')
  })

  it('stops offering to add at ten servers', () => {
    const ten = Array.from({ length: 10 }, (_, index) => ({ ...crm, id: `s${index}`, slug: `srv-${index}` }))
    const html = withServers(ten)

    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>.*Add a server/)
    expect(html).toContain('at most 10 servers')
  })

  it('gives each server its own address in the connection guide', () => {
    const html = withServers([crm, { ...crm, id: 's2', slug: 'erp', name: 'ERP', mcp_url: 'https://dep.example.com/mcp/erp' }])

    expect(html).toContain('Connect to')
    expect(html).toContain('https://dep.example.com/mcp')
  })

  it('does not offer a server that is switched off in the connection guide', () => {
    expect(withServers([{ ...crm, enabled: false }])).not.toContain('Connect to')
  })
})
