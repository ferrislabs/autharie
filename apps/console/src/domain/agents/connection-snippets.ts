export type ClientId = 'claude-code' | 'claude-desktop' | 'cursor' | 'vscode' | 'chatgpt'

export interface Step {
  text: string
  snippet?: string
}

export interface ClientGuide {
  id: ClientId
  label: string
  steps: Step[]
}

export interface ConnectionInput {
  mcpUrl: string
  serverName: string
}

export const FALLBACK_SERVER_NAME = 'autharie'

export const SIGN_IN_STEP =
  'Sign in in the browser when asked. The agent then only gets what your policies allow; watch the Activity tab.'

export function safeServerName(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug === '' ? FALLBACK_SERVER_NAME : slug
}

export function connectionGuides({ mcpUrl, serverName }: ConnectionInput): ClientGuide[] {
  const name = safeServerName(serverName)
  const json = (value: unknown) => JSON.stringify(value, null, 2)

  return [
    {
      id: 'claude-code',
      label: 'Claude Code',
      steps: [
        {
          text: 'Run this command in a terminal.',
          snippet: `claude mcp add --transport http ${name} ${mcpUrl}`,
        },
        { text: 'Start Claude Code and run /mcp to open the server and sign in.' },
        { text: SIGN_IN_STEP },
      ],
    },
    {
      id: 'claude-desktop',
      label: 'Claude Desktop / claude.ai',
      steps: [
        { text: 'Open Settings, then Connectors, then Add custom connector.' },
        { text: 'Paste the MCP URL.', snippet: mcpUrl },
        { text: SIGN_IN_STEP },
      ],
    },
    {
      id: 'cursor',
      label: 'Cursor',
      steps: [
        {
          text: 'Add this to ~/.cursor/mcp.json.',
          snippet: json({ mcpServers: { [name]: { url: mcpUrl } } }),
        },
        { text: 'Reload Cursor and enable the server in Settings, then MCP.' },
        { text: SIGN_IN_STEP },
      ],
    },
    {
      id: 'vscode',
      label: 'VS Code',
      steps: [
        {
          text: 'Add this to .vscode/mcp.json in your workspace.',
          snippet: json({ servers: { [name]: { type: 'http', url: mcpUrl } } }),
        },
        { text: 'Click Start above the server entry in the file.' },
        { text: SIGN_IN_STEP },
      ],
    },
    {
      id: 'chatgpt',
      label: 'ChatGPT',
      steps: [
        { text: 'Open Settings, then Connectors, and turn on developer mode.' },
        { text: 'Add a connector and paste the MCP URL.', snippet: mcpUrl },
        { text: SIGN_IN_STEP },
      ],
    },
  ]
}
