import { describe, expect, it } from 'vitest'
import { connectionGuides, safeServerName, SIGN_IN_STEP } from './connection-snippets'

const URL = 'https://dep.example.com/mcp'

function guide(id: string, serverName = 'My Deployment') {
  const found = connectionGuides({ mcpUrl: URL, serverName }).find((entry) => entry.id === id)
  if (!found) throw new Error('no guide')
  return found
}

describe('safeServerName', () => {
  it('slugs to kebab-case', () => {
    expect(safeServerName('  My Deployment_1! ')).toBe('my-deployment-1')
  })

  it('falls back when nothing usable is left', () => {
    expect(safeServerName('')).toBe('autharie')
    expect(safeServerName('???')).toBe('autharie')
  })
})

describe('connectionGuides', () => {
  it('covers the five clients', () => {
    expect(connectionGuides({ mcpUrl: URL, serverName: 'x' }).map((entry) => entry.id)).toEqual([
      'claude-code',
      'claude-desktop',
      'cursor',
      'vscode',
      'chatgpt',
    ])
  })

  it('builds the Claude Code command', () => {
    expect(guide('claude-code').steps[0].snippet).toBe(
      `claude mcp add --transport http my-deployment ${URL}`,
    )
  })

  it('builds the Cursor config', () => {
    const snippet = guide('cursor').steps[0].snippet ?? ''
    expect(JSON.parse(snippet)).toEqual({ mcpServers: { 'my-deployment': { url: URL } } })
  })

  it('builds the VS Code config', () => {
    const snippet = guide('vscode').steps[0].snippet ?? ''
    expect(JSON.parse(snippet)).toEqual({
      servers: { 'my-deployment': { type: 'http', url: URL } },
    })
  })

  it('gives the URL to copy for connector based clients', () => {
    expect(guide('claude-desktop').steps[1].snippet).toBe(URL)
    expect(guide('chatgpt').steps[1].snippet).toBe(URL)
  })

  it('ends every guide on the sign in step', () => {
    for (const entry of connectionGuides({ mcpUrl: URL, serverName: 'x' })) {
      expect(entry.steps[entry.steps.length - 1].text).toBe(SIGN_IN_STEP)
    }
  })
})
