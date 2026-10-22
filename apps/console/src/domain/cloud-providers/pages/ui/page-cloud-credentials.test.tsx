import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { CloudCredential } from '../../types/cloud-provider'
import { PageCloudCredentials } from './page-cloud-credentials'

const noop = () => {}

function credential(
  label: string,
  id = label,
  application_name?: string | null,
): CloudCredential {
  return {
    id,
    label,
    provider: 'scaleway',
    application_name,
    created_at: '2026-10-01T00:00:00Z',
  } as CloudCredential
}

function page(
  credentials: CloudCredential[],
  over: { unavailable?: string; deleteRefusal?: string } = {},
) {
  return renderToStaticMarkup(
    <PageCloudCredentials
      credentials={credentials}
      isLoading={false}
      isSaving={false}
      unavailable={over.unavailable}
      deleteRefusal={over.deleteRefusal}
      onRegister={noop}
      onDelete={noop}
      onDialogClosed={noop}
    />,
  )
}

describe('the button that connects a provider', () => {
  it('is named Connect provider, in the header and in the empty state', () => {
    const html = page([])
    expect(html.match(/Connect provider/g)).toHaveLength(2)
    expect(html).not.toContain('New credential')
    expect(html).not.toContain('aria-haspopup="menu"')
  })

  it('stays in the header once a provider is connected', () => {
    const html = page([credential('Scaleway production')])
    expect(html.match(/Connect provider/g)).toHaveLength(1)
  })
})

describe('the connected providers', () => {
  it('are cards, not a table', () => {
    const html = page([credential('Scaleway production')])
    expect(html).not.toContain('<table')
    expect(html).toContain('lg:grid-cols-3')
  })

  it('show the logo, the label, the provider, its description and a way to delete', () => {
    const html = page([credential('Scaleway production')])
    expect(html).toContain('aria-label="Scaleway"')
    expect(html).toContain('>Scaleway production</p>')
    expect(html).toContain('>Scaleway</p>')
    expect(html).toContain('Run your clusters in your own Scaleway project.')
    expect(html).toContain('aria-label="Delete Scaleway production"')
  })

  it('name the application the key belongs to in Scaleway, to tell accounts apart', () => {
    const html = page([credential('Scaleway production', 'a', 'autharie')])
    expect(html).toContain('Application')
    expect(html).toContain('>autharie</span>')
  })

  it('show the application as a badge', () => {
    const html = page([credential('Scaleway production', 'a', 'autharie')])
    expect(html).toMatch(
      /<span class="rounded-md border bg-muted\/50 px-1\.5 py-0\.5 font-mono text-foreground">autharie<\/span>/,
    )
  })

  it('say nothing about an application when the platform did not name one', () => {
    expect(page([credential('x', 'a', null)])).not.toContain('Application')
    expect(page([credential('x', 'b')])).not.toContain('Application')
  })

  it('give each credential its own card, sorted by label', () => {
    const html = page([credential('b-staging'), credential('a-production')])
    expect(html.match(/aria-label="Delete /g)).toHaveLength(2)
    expect(html.indexOf('a-production')).toBeLessThan(html.indexOf('b-staging'))
  })

  it('leave the empty state out', () => {
    expect(page([credential('x')])).not.toContain('No cloud account yet')
  })
})

describe('the refusal because customer cloud is off', () => {
  const message = 'customer cloud is not enabled on this installation'

  it('is shown in the page body, with the buttons that connect a provider still there', () => {
    const html = page([], { unavailable: message })
    expect(html).toContain(message)
    expect(html).toContain('role="alert"')
    expect(html.match(/Connect provider/g)).toHaveLength(2)
  })

  it('is not shown when nothing was refused', () => {
    expect(page([])).not.toContain('role="alert"')
  })
})
