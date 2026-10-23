import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KnownAgent } from '../../../registry'
import { AgentListInput, AgentValueInput } from './agent-picker'
import { KnownAgentsContext } from './known-agents'

const noop = () => {}

const known: KnownAgent[] = [
  { id: 'dcr-1', label: 'Claude Desktop', autonomous: false },
  { id: 'agent-nightly-abc123', label: 'Nightly report', autonomous: true },
]

function within(node: React.ReactNode) {
  return renderToStaticMarkup(
    <KnownAgentsContext.Provider value={known}>{node}</KnownAgentsContext.Provider>,
  )
}

describe('the agent pickers of the canvas', () => {
  it('lists the registry by name as options of the specific agents input', () => {
    const html = within(<AgentListInput id='clients' values={[]} onChange={noop} />)

    expect(html).toContain('<option value="dcr-1">Claude Desktop</option>')
    expect(html).toContain('<option value="agent-nightly-abc123">Nightly report</option>')
    expect(html).toContain('Pick an agent by name, or type a client id')
  })

  it('shows the chosen agents by name, and unknown ones as typed', () => {
    const html = within(
      <AgentListInput id='clients' values={['dcr-1', 'typed-by-hand']} onChange={noop} />,
    )

    expect(html).toContain('Claude Desktop')
    expect(html).toContain('typed-by-hand')
    expect(html).toContain('aria-label="Remove Claude Desktop"')
  })

  it('does not let a read-only editor remove an agent', () => {
    const html = within(<AgentListInput id='clients' values={['dcr-1']} disabled onChange={noop} />)

    expect(html).not.toContain('Remove ')
  })

  it('offers only autonomous agents for the autonomous agent subject, with its name', () => {
    const html = within(
      <AgentValueInput
        id='who'
        value='agent-nightly-abc123'
        placeholder='Pick'
        onChange={noop}
      />,
    )

    expect(html).toContain('<option value="agent-nightly-abc123">Nightly report</option>')
    expect(html).not.toContain('Claude Desktop')
    expect(html).toContain('<p class="text-xs text-muted-foreground">Nightly report</p>')
  })

  it('still works without any registry', () => {
    const html = renderToStaticMarkup(<AgentListInput id='clients' values={['x']} onChange={noop} />)

    expect(html).toContain('x')
    expect(html).not.toContain('<option')
  })
})
