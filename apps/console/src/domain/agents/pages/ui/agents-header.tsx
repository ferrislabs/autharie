import { Info } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { StatusBadge } from '@/components/ui/status-badge'
import { Switch } from '@/components/ui/switch'
import type { AgentsState } from '../../agent-state'
import type { AgentSettings } from '../../types'
import { CopyButton } from './copy-button'

interface Props {
  settings: AgentSettings
  state: AgentsState
  realms: string[] | null
  realm: string
  canManage: boolean
  isSaving: boolean
  error?: string
  onRealmChange: (realm: string) => void
  onToggle: (enabled: boolean) => void
  onSaveRealm: () => void
  onOpenOrganisationSettings: () => void
}

export function AgentsHeader({
  settings,
  state,
  realms,
  realm,
  canManage,
  isSaving,
  error,
  onRealmChange,
  onToggle,
  onSaveRealm,
  onOpenOrganisationSettings,
}: Props) {
  const realmDirty = realm.trim() !== settings.realm
  const realmLocked = !canManage || state.kind === 'organisation-off'
  const canSwitch = !state.switchDisabled && !isSaving && (settings.enabled || realm.trim() !== '')

  return (
    <div className='space-y-4 rounded-lg border bg-card p-5'>
      {state.callout && (
        <div
          role='note'
          className='flex items-start gap-3 rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'
        >
          <Info className='mt-0.5 h-4 w-4 shrink-0' />
          <div className='space-y-1'>
            <p>{state.callout}</p>
            <button
              type='button'
              className='font-medium underline underline-offset-4'
              onClick={onOpenOrganisationSettings}
            >
              Open organisation settings
            </button>
          </div>
        </div>
      )}

      <div className='flex items-center justify-between gap-4'>
        <div className='space-y-0.5'>
          <Label htmlFor='agents-enabled' className='text-base font-semibold'>
            Agents on this deployment
          </Label>
          <p className='text-sm text-muted-foreground'>
            Let AI agents reach this deployment over MCP. Each call is decided by the policies below.
          </p>
        </div>
        <div className='flex items-center gap-3'>
          <StatusBadge tone={state.badge.tone}>{state.badge.label}</StatusBadge>
          <Switch
            id='agents-enabled'
            checked={settings.enabled && state.kind !== 'organisation-off'}
            disabled={!canSwitch}
            onCheckedChange={onToggle}
          />
        </div>
      </div>

      <div className='grid gap-4 sm:grid-cols-2'>
        <div className='space-y-2'>
          <Label htmlFor='agents-realm'>Realm</Label>
          <div className='flex items-center gap-2'>
            {realms !== null && realms.length > 0 ? (
              <Select value={realm} onValueChange={onRealmChange} disabled={realmLocked}>
                <SelectTrigger id='agents-realm' className='w-full'>
                  <SelectValue placeholder='Pick a realm' />
                </SelectTrigger>
                <SelectContent>
                  {realms.map((entry) => (
                    <SelectItem key={entry} value={entry}>
                      {entry}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <Input
                id='agents-realm'
                value={realm}
                disabled={realmLocked}
                placeholder='master'
                autoComplete='off'
                spellCheck={false}
                onChange={(event) => onRealmChange(event.target.value)}
              />
            )}
            {realmDirty && settings.realm !== '' && !realmLocked && (
              <Button size='sm' onClick={onSaveRealm} disabled={isSaving || realm.trim() === ''}>
                Save
              </Button>
            )}
          </div>
          <p className='text-xs text-muted-foreground'>The realm whose rules decide each call.</p>
        </div>

        <div className='space-y-2'>
          <Label htmlFor='agents-mcp-url'>MCP URL</Label>
          <div className='flex items-center gap-2'>
            <Input
              id='agents-mcp-url'
              readOnly
              value={settings.mcp_url ?? ''}
              placeholder='Shown once agents are ready'
              className='font-mono text-xs'
            />
            {settings.mcp_url && <CopyButton text={settings.mcp_url} label='Copy the MCP URL' />}
          </div>
          <p className='text-xs text-muted-foreground'>
            Give this address to an MCP client. It signs in with this deployment’s realm.
          </p>
        </div>
      </div>

      <dl className='flex flex-wrap gap-x-8 gap-y-1 text-sm'>
        <div className='flex gap-2'>
          <dt className='text-muted-foreground'>Status</dt>
          <dd className='font-medium'>{state.badge.label}</dd>
        </div>
        <div className='flex gap-2'>
          <dt className='text-muted-foreground'>Gateway client</dt>
          <dd className='font-medium'>{state.gateway}</dd>
        </div>
      </dl>

      {state.message && (
        <p
          role={state.kind === 'failed' ? 'alert' : undefined}
          className={
            state.kind === 'failed' ? 'text-sm text-destructive' : 'text-sm text-muted-foreground'
          }
        >
          {state.message}
        </p>
      )}

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}
    </div>
  )
}
