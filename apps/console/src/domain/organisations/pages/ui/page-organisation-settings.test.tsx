import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PageOrganisationSettings } from './page-organisation-settings'

const noop = () => {}

const props = {
  mcpEnabled: false,
  canManage: true,
  isLoading: false,
  isSaving: false,
  onMcpEnabledChange: noop,
}

function theSwitch(html: string): string {
  return html.match(/<button[^>]*role="switch"[^>]*>/)![0]
}

const DISABLED = /\sdisabled=""/

describe('the organisation settings page', () => {
  it('offers the AI switch with a line saying what it does', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} />)
    expect(html).toContain('>Allow AI agents (MCP)</label>')
    expect(html).toContain('Lets AI assistants act on this organisation')
  })

  it('shows the switch off when the organisation has not allowed agents', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} mcpEnabled={false} />)
    expect(theSwitch(html)).toContain('aria-checked="false"')
  })

  it('shows the switch on when the organisation allows agents', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} mcpEnabled />)
    expect(theSwitch(html)).toContain('aria-checked="true"')
  })

  it('leaves the switch usable for a member who manages the organisation', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} />)
    expect(theSwitch(html)).not.toMatch(DISABLED)
    expect(html).not.toContain('Only members who manage the organisation')
  })

  it('disables the switch for a member who does not, and says why', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} canManage={false} />)
    expect(theSwitch(html)).toMatch(DISABLED)
    expect(html).toContain('Only members who manage the organisation can change this.')
  })

  it('disables the switch while a change is being saved', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} isSaving />)
    expect(theSwitch(html)).toMatch(DISABLED)
  })

  it('shows why a change was refused', () => {
    const html = renderToStaticMarkup(
      <PageOrganisationSettings {...props} refusal='forbidden: insufficient permissions' />,
    )
    expect(html).toContain('forbidden: insufficient permissions')
  })

  it('shows only a skeleton during the first load', () => {
    const html = renderToStaticMarkup(<PageOrganisationSettings {...props} isLoading />)
    expect(html).toContain('data-slot="skeleton"')
    expect(html).not.toContain('role="switch"')
  })
})
