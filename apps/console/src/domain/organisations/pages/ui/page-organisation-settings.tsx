import { Page, PageTitle, Section } from '@/components/layout/page'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'

interface Props {
  mcpEnabled: boolean
  canManage: boolean
  isLoading: boolean
  isSaving: boolean
  refusal?: string
  onMcpEnabledChange: (enabled: boolean) => void
}

export function PageOrganisationSettings({
  mcpEnabled,
  canManage,
  isLoading,
  isSaving,
  refusal,
  onMcpEnabledChange,
}: Props) {
  if (isLoading) {
    return (
      <Page>
        <Skeleton className='h-64 w-full' />
      </Page>
    )
  }

  return (
    <Page>
      <PageTitle title='Settings' />

      <div className='mt-6 space-y-8'>
        <Section title='AI agents'>
          <div className='flex items-start justify-between gap-6'>
            <div className='space-y-1'>
              <Label htmlFor='organisation-mcp-enabled'>Allow AI agents (MCP)</Label>
              <p className='text-sm text-muted-foreground'>
                Lets AI assistants act on this organisation for the people who sign in to them.
              </p>
              {!canManage && (
                <p className='text-sm text-muted-foreground'>
                  Only members who manage the organisation can change this.
                </p>
              )}
            </div>
            <Switch
              id='organisation-mcp-enabled'
              checked={mcpEnabled}
              disabled={!canManage || isSaving}
              onCheckedChange={onMcpEnabledChange}
            />
          </div>
          {refusal && (
            <p role='alert' className='text-sm text-destructive'>
              {refusal}
            </p>
          )}
        </Section>
      </div>
    </Page>
  )
}
