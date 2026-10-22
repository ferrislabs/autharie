import { Section } from '@/components/layout/page'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import type { AgentsState } from '../../agent-state'
import { connectionGuides, FALLBACK_SERVER_NAME } from '../../connection-snippets'
import type { AgentSettings } from '../../types'
import { CopyButton } from './copy-button'

interface Props {
  settings: AgentSettings
  state: AgentsState
}

function missing(settings: AgentSettings, state: AgentsState): string {
  if (state.callout) return state.callout
  if (!settings.enabled) return 'Turn agents on for this deployment above to get a connection address.'
  return state.message ?? 'The MCP URL appears here once the deployment has applied the change.'
}

export function ConnectAgent({ settings, state }: Props) {
  const mcpUrl = settings.mcp_url
  const guides = mcpUrl ? connectionGuides({ mcpUrl, serverName: FALLBACK_SERVER_NAME }) : []

  return (
    <Section title='Connect an agent'>
      {settings.enabled && mcpUrl ? (
        <Tabs defaultValue='claude-code' className='gap-3'>
          <TabsList className='h-auto flex-wrap justify-start'>
            {guides.map((guide) => (
              <TabsTrigger key={guide.id} value={guide.id}>
                {guide.label}
              </TabsTrigger>
            ))}
          </TabsList>
          {guides.map((guide) => (
            <TabsContent key={guide.id} value={guide.id}>
              <ol className='space-y-3 rounded-lg border bg-card p-4 text-sm'>
                {guide.steps.map((step, index) => (
                  <li key={step.text} className='flex gap-3'>
                    <span className='flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium'>
                      {index + 1}
                    </span>
                    <div className='min-w-0 flex-1 space-y-2'>
                      <p>{step.text}</p>
                      {step.snippet && (
                        <div className='flex items-start gap-2'>
                          <pre className='min-w-0 flex-1 overflow-x-auto rounded-md border bg-muted/40 p-2 font-mono text-xs'>
                            {step.snippet}
                          </pre>
                          <CopyButton text={step.snippet} label={`Copy the ${guide.label} snippet`} />
                        </div>
                      )}
                    </div>
                  </li>
                ))}
              </ol>
            </TabsContent>
          ))}
        </Tabs>
      ) : (
        <p className='text-sm text-muted-foreground'>{missing(settings, state)}</p>
      )}
    </Section>
  )
}
