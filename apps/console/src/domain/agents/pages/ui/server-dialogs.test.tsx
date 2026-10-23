import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { AgentPolicy, AgentServer } from '../../types'
import { ReferringPolicies } from './remove-server-dialog'
import { ServerForm } from './server-dialog'

const noop = () => {}

const server: AgentServer = {
  id: 's1',
  slug: 'crm',
  name: 'CRM',
  url: 'https://tools.example.com/mcp',
  enabled: true,
  created_at: '',
  updated_at: '',
}

function form(over: Partial<React.ComponentProps<typeof ServerForm>> = {}) {
  return renderToStaticMarkup(
    <ServerForm
      server={null}
      taken={[]}
      isSaving={false}
      onClose={noop}
      onSave={noop}
      {...over}
    />,
  )
}

function policy(id: string, name: string): AgentPolicy {
  return { id, name, enabled: true, cedar: '', created_at: '', updated_at: '' }
}

describe('the form that registers a server', () => {
  it('asks for a name, a slug and a public https URL, and cannot be sent empty', () => {
    const html = form()

    expect(html).toContain('id="server-name"')
    expect(html).toContain('id="server-slug"')
    expect(html).toContain('id="server-url"')
    expect(html).toContain('https://tools.example.com/mcp')
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Add server/)
  })

  it('shows the server being edited, with a slug that cannot change', () => {
    const html = form({ server })

    expect(html).toContain('value="CRM"')
    expect(html).toContain('value="https://tools.example.com/mcp"')
    expect(html).toMatch(/<input[^>]*id="server-slug"[^>]*disabled=""/)
    expect(html).toContain('Save server')
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*>Save server/)
  })

  it('refuses a server whose URL is not public https, saying why', () => {
    const html = form({ server: { ...server, url: 'http://10.0.0.5/mcp' } })

    expect(html).toContain('The URL must start with https://.')
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Save server/)
  })

  it('shows a save error and a saving state', () => {
    const html = form({ server, error: 'The slug is taken', isSaving: true })

    expect(html).toContain('The slug is taken')
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>.*Save server/)
  })
})

describe('the policies a removal leaves behind', () => {
  it('names each of them and says they stay', () => {
    const html = renderToStaticMarkup(
      <ReferringPolicies policies={[policy('a', 'CRM reads'), policy('b', 'No deletes')]} slug='crm' />,
    )

    expect(html).toContain('These policies refer')
    expect(html).toContain('CRM reads')
    expect(html).toContain('No deletes')
    expect(html).toContain('unknown server')
  })

  it('says nothing when no policy refers to the server', () => {
    expect(renderToStaticMarkup(<ReferringPolicies policies={[]} slug='crm' />)).toBe('')
  })
})
