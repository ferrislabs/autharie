import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { AgentClient, Registry } from '../../types'
import { AgentRegistry } from './agent-registry'
import { confirmCopy } from '../../agent-actions'

const noop = () => {}
const NOW = new Date('2026-10-23T12:00:00Z')

function client(over: Partial<AgentClient> = {}): AgentClient {
  return {
    client_id: 'dcr-1',
    name: 'Claude Desktop',
    source: 'dynamic',
    kind: 'delegated',
    access: 'approved',
    last_seen_at: '2026-10-23T11:55:00Z',
    ...over,
  }
}

function registry(clients: AgentClient[], over: Partial<Registry> = {}): Registry {
  return { approval_required: false, data: clients, truncated: false, ...over }
}

function render(
  over: Partial<React.ComponentProps<typeof AgentRegistry>> = {},
  clients: AgentClient[] = [client()],
) {
  return renderToStaticMarkup(
    <AgentRegistry
      registry={registry(clients)}
      isLoading={false}
      canManage
      filter='all'
      approvalSaving={false}
      busyClient={null}
      now={NOW}
      onFilter={noop}
      onApprovalRequired={noop}
      onApprove={noop}
      onUnblock={noop}
      onBlock={noop}
      onDelete={noop}
      onRotate={noop}
      onNewAutonomous={noop}
      {...over}
    />,
  )
}

describe('the agent registry', () => {
  it('lists each agent with its name, client id, source, kind, access and last activity', () => {
    const html = render({}, [
      client({ client_id: 'https://app.example/c.json', source: 'metadata_document' }),
    ])

    expect(html).toContain('Claude Desktop')
    expect(html).toContain('https://app.example/c.json')
    expect(html).toContain('Metadata document')
    expect(html).toContain('Delegated')
    expect(html).toContain('Approved')
    expect(html).toContain('5 minutes ago')
  })

  it('says an agent never called anything', () => {
    expect(render({}, [client({ last_seen_at: undefined })])).toContain('Never seen')
  })

  it('counts every filter and shows the pending ones when asked', () => {
    const clients = [
      client({ client_id: 'a', name: 'Alpha', access: 'pending' }),
      client({ client_id: 'b', name: 'Beta' }),
      client({ client_id: 'c', name: 'Gamma', access: 'blocked' }),
    ]

    const html = render({ filter: 'pending' }, clients)

    expect(html).toContain('Alpha')
    expect(html).not.toContain('Beta')
    expect(html).not.toContain('Gamma')
    expect(html).toMatch(/Pending<span[^>]*>1<\/span>/)
    expect(html).toMatch(/All<span[^>]*>3<\/span>/)
    expect(html).toMatch(/Blocked<span[^>]*>1<\/span>/)
  })

  it('offers Approve and Block on a pending agent, Unblock on a blocked one', () => {
    const pending = render({}, [client({ access: 'pending' })])
    const blocked = render({}, [client({ access: 'blocked' })])

    expect(pending).toContain('>Approve<')
    expect(pending).toContain('>Block<')
    expect(pending).not.toContain('>Unblock<')
    expect(blocked).toContain('>Unblock<')
    expect(blocked).not.toContain('>Block<')
    expect(pending).toContain('>Delete<')
  })

  it('offers to rotate only an agent created here', () => {
    const created = render({}, [client({ source: 'autharie', kind: 'autonomous' })])

    expect(created).toContain('Rotate secret')
    expect(created).toContain('Autonomous')
    expect(render()).not.toContain('Rotate secret')
  })

  it('does not offer to delete an agent only seen in calls', () => {
    expect(render({}, [client({ source: 'seen' })])).not.toContain('>Delete<')
  })

  it('shows no action and no creation button to someone who cannot manage', () => {
    const html = render({ canManage: false }, [client({ access: 'pending' })])

    expect(html).not.toContain('>Approve<')
    expect(html).not.toContain('New autonomous agent')
  })

  it('has a creation button for someone who can manage', () => {
    expect(render()).toContain('New autonomous agent')
  })

  it('explains the approval switch in one line and reflects the mode', () => {
    const off = render()
    const on = renderToStaticMarkup(
      <AgentRegistry
        registry={registry([client()], { approval_required: true })}
        isLoading={false}
        canManage
        filter='all'
        approvalSaving={false}
        busyClient={null}
        now={NOW}
        onFilter={noop}
        onApprovalRequired={noop}
        onApprove={noop}
        onUnblock={noop}
        onBlock={noop}
        onDelete={noop}
        onRotate={noop}
        onNewAutonomous={noop}
      />,
    )

    expect(off).toContain('Require approval for new agents')
    expect(off).toContain('refused at the gateway until you approve it')
    expect(off).toMatch(/id="approval-required"[^>]*aria-checked="false"|aria-checked="false"[^>]*id="approval-required"/)
    expect(on).toMatch(/id="approval-required"[^>]*aria-checked="true"|aria-checked="true"[^>]*id="approval-required"/)
  })

  it('locks the approval switch for someone who cannot manage', () => {
    const html = render({ canManage: false })

    expect(html).toMatch(/role="switch"[^>]*disabled=""|disabled=""[^>]*role="switch"/)
  })

  it('says when the registry holds more agents than it lists', () => {
    const html = renderToStaticMarkup(
      <AgentRegistry
        registry={registry([client()], { truncated: true })}
        isLoading={false}
        canManage
        filter='all'
        approvalSaving={false}
        busyClient={null}
        now={NOW}
        onFilter={noop}
        onApprovalRequired={noop}
        onApprove={noop}
        onUnblock={noop}
        onBlock={noop}
        onDelete={noop}
        onRotate={noop}
        onNewAutonomous={noop}
      />,
    )

    expect(html).toContain('more agents than the 500 newest')
  })

  it('explains an empty registry and an empty filter differently', () => {
    expect(render({}, [])).toContain('No agent yet')
    expect(render({ filter: 'blocked' }, [client()])).toContain('No agent matches this filter')
  })

  it('shows an error without hiding the list', () => {
    const html = render({ error: 'boom' })

    expect(html).toContain('boom')
    expect(html).toContain('Claude Desktop')
  })

  it('shows a skeleton while loading', () => {
    expect(render({ isLoading: true })).not.toContain('Require approval')
  })

  it('uses theme tokens so it reads in light and dark mode', () => {
    const html = render({}, [client({ access: 'pending' })])

    expect(html).not.toMatch(/bg-white|text-black|bg-gray-/)
  })
})

describe('the confirmation before blocking or deleting', () => {
  it('says a block refuses the agent at the gateway and disables its client', () => {
    const copy = confirmCopy({ kind: 'block', client: client() })

    expect(copy.title).toBe('Block Claude Desktop?')
    expect(copy.body).toContain('refuses every call')
    expect(copy.body).toContain('disabled in the realm')
    expect(copy.confirm).toBe('Block agent')
  })

  it('says a delete removes the client and that it can register again', () => {
    const copy = confirmCopy({ kind: 'delete', client: client({ name: undefined }) })

    expect(copy.title).toBe('Delete dcr-1?')
    expect(copy.body).toContain('deleted from the realm')
    expect(copy.confirm).toBe('Delete agent')
  })
})
