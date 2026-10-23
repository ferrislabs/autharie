import { useEffect, useMemo, useState } from 'react'
import { ArrowLeft, Palette } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { StatusBadge } from '@/components/ui/status-badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { describeUnavailable } from '../../branding-plan'
import { PANELS, panelsWithProblems, type PanelId } from '../../theme-panels'
import { ACTIVE_BADGE, themeStateBadge } from '../../theme-state'
import {
  describeNameProblem,
  isDirty,
  isValid,
  setAt,
  validateTokens,
  withDefaults,
  type ThemeDraft,
} from '../../theme-tokens'
import { Alert, Notice } from './components/alert'
import { PanelBody } from './components/builder-panels'
import { ThemePreview } from './components/theme-preview'
import type { PanelContext } from './components/theme-controls'

interface Props {
  isLoading: boolean
  unsupported: boolean
  error?: string
  theme: Schemas.ThemeView | null
  state: Schemas.ThemeSync | null
  planOpen: boolean
  tier?: string
  canManage: boolean
  isSaving: boolean
  saveError?: string
  onSave: (draft: ThemeDraft, activate: boolean) => void
  onBack: () => void
  onDirtyChange: (dirty: boolean) => void
}

export function PageThemeBuilder({
  isLoading,
  unsupported,
  error,
  theme,
  state,
  planOpen,
  tier,
  canManage,
  isSaving,
  saveError,
  onSave,
  onBack,
  onDirtyChange,
}: Props) {
  if (unsupported) {
    return (
      <div className='space-y-6'>
        <EmptyState
          icon={<Palette className='h-5 w-5' />}
          title='Login pages are available on FerrisKey deployments'
          description='This deployment does not run FerrisKey, so there is no login theme to build.'
          action={<BackButton onBack={onBack} />}
        />
      </div>
    )
  }

  if (isLoading) return <Skeleton className='h-96 w-full' />

  if (!theme) {
    return (
      <div className='space-y-4'>
        {error && <Alert>{error}</Alert>}
        <EmptyState
          icon={<Palette className='h-5 w-5' />}
          title='Theme not found'
          description='It may have been deleted, or it belongs to another realm.'
          action={<BackButton onBack={onBack} />}
        />
      </div>
    )
  }

  return (
    <Builder
      key={`${theme.id}:${theme.updated_at}`}
      theme={theme}
      state={state}
      planOpen={planOpen}
      tier={tier}
      canManage={canManage}
      isSaving={isSaving}
      error={error}
      saveError={saveError}
      onSave={onSave}
      onBack={onBack}
      onDirtyChange={onDirtyChange}
    />
  )
}

function BackButton({ onBack }: { onBack: () => void }) {
  return (
    <Button variant='ghost' size='sm' onClick={onBack}>
      <ArrowLeft className='h-3.5 w-3.5' />
      Back to themes
    </Button>
  )
}

function Builder({
  theme,
  state,
  planOpen,
  tier,
  canManage,
  isSaving,
  error,
  saveError,
  onSave,
  onBack,
  onDirtyChange,
}: Pick<
  Props,
  | 'state'
  | 'planOpen'
  | 'tier'
  | 'canManage'
  | 'isSaving'
  | 'error'
  | 'saveError'
  | 'onSave'
  | 'onBack'
  | 'onDirtyChange'
> & { theme: Schemas.ThemeView }) {
  const [draft, setDraft] = useState<ThemeDraft>(() => ({
    name: theme.name,
    tokens: withDefaults(theme.tokens),
  }))
  const [panel, setPanel] = useState<PanelId>('colors')

  const problems = useMemo(() => validateTokens(draft.tokens), [draft.tokens])
  const flagged = useMemo(() => panelsWithProblems(problems), [problems])
  const nameProblem = describeNameProblem(draft.name)
  const valid = isValid(draft)
  const dirty = isDirty(draft, theme)
  const canMutate = canManage && planOpen
  const canSave = canMutate && !isSaving && dirty && valid
  const canActivate = canMutate && !isSaving && valid
  const badge = theme.active && state ? themeStateBadge(state) : null

  useEffect(() => {
    onDirtyChange(dirty)
  }, [dirty, onDirtyChange])

  const ctx: PanelContext = {
    tokens: draft.tokens,
    problems,
    readOnly: !canMutate,
    set: (path, value) =>
      setDraft((current) => ({ ...current, tokens: setAt(current.tokens, path, value) })),
  }

  return (
    <div className='space-y-6'>
      <div className='flex flex-wrap items-start justify-between gap-4 border-b pb-4'>
        <div className='flex min-w-0 items-start gap-3'>
          <BackButton onBack={onBack} />
          <div className='space-y-1'>
            <h1 className='sr-only'>Theme builder</h1>
            <Label htmlFor='theme-name' className='sr-only'>
              Theme name
            </Label>
            <Input
              id='theme-name'
              value={draft.name}
              readOnly={!canMutate}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              aria-invalid={!!nameProblem}
              className='h-9 w-72 text-base font-semibold'
            />
            {nameProblem && <p className='text-xs text-destructive'>{nameProblem}</p>}
          </div>
          <div className='flex items-center gap-2 pt-1.5'>
            {theme.active && (
              <StatusBadge tone={ACTIVE_BADGE.tone}>{ACTIVE_BADGE.label}</StatusBadge>
            )}
            {badge && <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>}
          </div>
        </div>

        {canManage && (
          <div className='flex shrink-0 items-center gap-2'>
            <Button variant='outline' disabled={!canSave} onClick={() => onSave(draft, false)}>
              {isSaving && <Spinner className='size-3.5' />}
              {isSaving ? 'Saving' : 'Save'}
            </Button>
            {!theme.active && (
              <Button disabled={!canActivate} onClick={() => onSave(draft, true)}>
                Save and activate
              </Button>
            )}
          </div>
        )}
      </div>

      {!planOpen && (
        <Notice title='Branding is not included in your plan'>{describeUnavailable(tier)}</Notice>
      )}
      {error && <Alert>{error}</Alert>}
      {saveError && <Alert>{saveError}</Alert>}
      {badge?.message && <Alert>{badge.message}</Alert>}

      <div className='flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground'>
        {theme.active && <span>This theme is active: saving it sends it to FerrisKey.</span>}
        {dirty && <span>You have unsaved changes.</span>}
        {Object.keys(problems).length > 0 && (
          <span className='text-destructive'>Fix the highlighted values to save.</span>
        )}
      </div>

      <div className='grid items-start gap-6 lg:grid-cols-[24rem_minmax(0,1fr)]'>
        <div className='rounded-lg border p-4'>
          <Tabs value={panel} onValueChange={(next) => setPanel(next as PanelId)}>
            <TabsList className='h-auto w-full flex-wrap justify-start gap-1'>
              {PANELS.map((entry) => (
                <TabsTrigger key={entry.id} value={entry.id}>
                  {entry.label}
                  {flagged.has(entry.id) && (
                    <span
                      role='img'
                      aria-label='has a value to fix'
                      className='ml-1.5 size-1.5 rounded-full bg-destructive'
                    />
                  )}
                </TabsTrigger>
              ))}
            </TabsList>
            {PANELS.map((entry) => (
              <TabsContent key={entry.id} value={entry.id} className='mt-4'>
                <PanelBody
                  id={entry.id}
                  ctx={ctx}
                  onReset={(tokens) => setDraft((current) => ({ ...current, tokens }))}
                />
              </TabsContent>
            ))}
          </Tabs>
        </div>

        <div className='space-y-2 lg:sticky lg:top-4'>
          <p className='text-sm font-medium'>Preview</p>
          <ThemePreview tokens={draft.tokens} />
        </div>
      </div>
    </div>
  )
}
