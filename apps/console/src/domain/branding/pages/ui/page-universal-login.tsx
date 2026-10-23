import { useState } from 'react'
import { Copy, Download, FileUp, LayoutTemplate, Plus, Trash2, Undo2 } from 'lucide-react'
import type { Schemas } from '@/api/api.client'
import { EmptyState, Section, SectionPage } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { StatusBadge } from '@/components/ui/status-badge'
import { describeUnavailable } from '../../branding-plan'
import { MAX_FILE_BYTES, parseImport, type ThemeFile } from '../../theme-files'
import { limitMessage, limitReached } from '../../theme-limit'
import { ACTIVE_BADGE, themeStateBadge, themeStateSummary } from '../../theme-state'
import { describeNameProblem, hasLegacyBranding } from '../../theme-tokens'
import { Alert, Notice } from './components/alert'
import { ConfirmDialog } from './components/confirm-dialog'
import { RealmPicker } from './realm-picker'

export const DELETE_ACTIVE_HINT =
  'The active theme cannot be deleted. Activate another theme or use FerrisKey’s default look first.'

const DESCRIPTION = 'The sign-in pages of this realm, as themes you can build and switch between.'

const updated = new Intl.DateTimeFormat('en', { dateStyle: 'medium' })

interface Props {
  isLoading: boolean
  unsupported: boolean
  error?: string
  actionError?: string
  realm: string
  realms: string[]
  onRealmChange: (realm: string) => void
  themes: Schemas.ThemeView[]
  state: Schemas.ThemeSync | null
  planOpen: boolean
  tier?: string
  canManage: boolean
  isBusy: boolean
  legacy: Schemas.Branding | null
  onCreate: (name: string) => void
  onImport: (theme: ThemeFile) => void
  onImportLegacy: () => void
  onEdit: (theme: Schemas.ThemeView) => void
  onActivate: (theme: Schemas.ThemeView) => void
  onDuplicate: (theme: Schemas.ThemeView) => void
  onExport: (theme: Schemas.ThemeView) => void
  onDelete: (theme: Schemas.ThemeView) => void
  onDeactivate: () => void
}

export function PageUniversalLogin({
  isLoading,
  unsupported,
  error,
  realm,
  realms,
  onRealmChange,
  ...body
}: Props) {
  if (unsupported) {
    return (
      <SectionPage title='Universal login' description={DESCRIPTION}>
        <EmptyState
          icon={<LayoutTemplate className='h-5 w-5' />}
          title='Login pages are available on FerrisKey deployments'
          description='This deployment does not run FerrisKey, so there is no realm to theme the login pages of.'
        />
      </SectionPage>
    )
  }

  return (
    <SectionPage
      title='Universal login'
      description={DESCRIPTION}
      actions={<RealmPicker realm={realm} realms={realms} onChange={onRealmChange} />}
    >
      {error && <Alert>{error}</Alert>}

      {isLoading ? (
        <Skeleton className='h-64 w-full' />
      ) : (
        <ThemesBody key={realm} realm={realm} {...body} />
      )}
    </SectionPage>
  )
}

function ThemesBody({
  realm,
  actionError,
  themes,
  state,
  planOpen,
  tier,
  canManage,
  isBusy,
  legacy,
  onCreate,
  onImport,
  onImportLegacy,
  onEdit,
  onActivate,
  onDuplicate,
  onExport,
  onDelete,
  onDeactivate,
}: Omit<Props, 'isLoading' | 'unsupported' | 'error' | 'realms' | 'onRealmChange'>) {
  const [creating, setCreating] = useState(false)
  const [importing, setImporting] = useState(false)
  const [deactivating, setDeactivating] = useState(false)
  const [deleting, setDeleting] = useState<Schemas.ThemeView | null>(null)

  const active = themes.find((theme) => theme.active) ?? null
  const full = limitReached(themes.length)
  const canMutate = canManage && planOpen
  const canAdd = canMutate && !isBusy && !full
  const showLegacy = realm === 'master' && themes.length === 0 && hasLegacyBranding(legacy)
  const badge = state ? themeStateBadge(state) : null

  return (
    <>
      {!planOpen && (
        <Notice title='Branding is not included in your plan'>{describeUnavailable(tier)}</Notice>
      )}
      {actionError && <Alert>{actionError}</Alert>}

      {badge && (
        <div className='overflow-hidden rounded-lg border'>
          <div className='flex items-center gap-2 border-b bg-muted/30 px-4 py-2.5'>
            <span className='text-sm font-medium'>{themeStateSummary(active?.name ?? null)}</span>
            <StatusBadge tone={badge.tone}>{badge.label}</StatusBadge>
          </div>
          {badge.message && <p className='px-4 pt-2.5 text-sm text-destructive'>{badge.message}</p>}
          <p className='px-4 py-2.5 text-xs text-muted-foreground'>
            Applied means FerrisKey accepted the tokens, not that anyone looked at the login page.
          </p>
        </div>
      )}

      {showLegacy && (
        <div className='flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-muted/30 px-4 py-3'>
          <div className='space-y-0.5'>
            <p className='text-sm font-medium'>This deployment has branding from the old page</p>
            <p className='text-sm text-muted-foreground'>
              Its colors and radius still apply to the master realm. Import them as a theme to keep
              them and to edit the rest.
            </p>
          </div>
          {canManage && (
            <Button variant='outline' disabled={!canMutate || isBusy} onClick={onImportLegacy}>
              Import your current branding as a theme
            </Button>
          )}
        </div>
      )}

      <Section
        title='Themes'
        aside={
          canManage && (
            <div className='flex flex-wrap items-center justify-end gap-2'>
              {active && (
                <Button
                  variant='outline'
                  size='sm'
                  disabled={!canMutate || isBusy}
                  onClick={() => setDeactivating(true)}
                >
                  <Undo2 className='h-3.5 w-3.5' />
                  Use FerrisKey’s default look
                </Button>
              )}
              <Button
                variant='outline'
                size='sm'
                disabled={!canAdd}
                onClick={() => setImporting(true)}
              >
                <FileUp className='h-3.5 w-3.5' />
                Import JSON
              </Button>
              <Button size='sm' disabled={!canAdd} onClick={() => setCreating(true)}>
                <Plus className='h-3.5 w-3.5' />
                New theme
              </Button>
            </div>
          )
        }
      >
        {themes.length === 0 ? (
          <EmptyState
            icon={<LayoutTemplate className='h-5 w-5' />}
            title='No themes yet'
            description='Without an active theme FerrisKey uses its default look on this realm’s login pages. Create a theme to make them yours.'
            action={
              canManage ? (
                <Button disabled={!canAdd} onClick={() => setCreating(true)}>
                  <Plus className='h-3.5 w-3.5' />
                  Create your first theme
                </Button>
              ) : (
                <p className='text-sm text-muted-foreground'>
                  Ask someone who can manage instances to create one.
                </p>
              )
            }
          />
        ) : (
          <ul className='divide-y rounded-lg border'>
            {themes.map((theme) => (
              <li
                key={theme.id}
                className='flex flex-wrap items-center justify-between gap-3 px-4 py-3'
              >
                <div className='min-w-0 space-y-0.5'>
                  <div className='flex items-center gap-2'>
                    <p className='truncate font-medium'>{theme.name}</p>
                    {theme.active && (
                      <StatusBadge tone={ACTIVE_BADGE.tone}>{ACTIVE_BADGE.label}</StatusBadge>
                    )}
                  </div>
                  <p className='text-xs text-muted-foreground'>
                    Updated {updated.format(new Date(theme.updated_at))}
                  </p>
                  {theme.active && canManage && (
                    <p className='text-xs text-muted-foreground'>{DELETE_ACTIVE_HINT}</p>
                  )}
                </div>

                <div className='flex flex-wrap items-center gap-1.5'>
                  <Button variant='outline' size='sm' onClick={() => onEdit(theme)}>
                    {canManage ? 'Edit' : 'View'}
                  </Button>
                  {canManage && !theme.active && (
                    <Button
                      variant='outline'
                      size='sm'
                      disabled={!canMutate || isBusy}
                      onClick={() => onActivate(theme)}
                    >
                      Activate
                    </Button>
                  )}
                  {canManage && (
                    <Button
                      variant='ghost'
                      size='sm'
                      disabled={!canAdd}
                      onClick={() => onDuplicate(theme)}
                    >
                      <Copy className='h-3.5 w-3.5' />
                      Duplicate
                    </Button>
                  )}
                  <Button variant='ghost' size='sm' onClick={() => onExport(theme)}>
                    <Download className='h-3.5 w-3.5' />
                    Export JSON
                  </Button>
                  {canManage && (
                    <Button
                      variant='ghost'
                      size='sm'
                      disabled={!canMutate || isBusy || theme.active}
                      title={theme.active ? DELETE_ACTIVE_HINT : undefined}
                      onClick={() => setDeleting(theme)}
                    >
                      <Trash2 className='h-3.5 w-3.5' />
                      Delete
                    </Button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}

        <p className='text-xs text-muted-foreground'>{limitMessage(themes.length)}</p>
      </Section>

      <NewThemeDialog
        open={creating}
        onOpenChange={setCreating}
        taken={themes.map((theme) => theme.name)}
        onCreate={(name) => {
          setCreating(false)
          onCreate(name)
        }}
      />
      <ImportDialog
        open={importing}
        onOpenChange={setImporting}
        onImport={(theme) => {
          setImporting(false)
          onImport(theme)
        }}
      />
      <ConfirmDialog
        open={deactivating}
        onOpenChange={setDeactivating}
        title='Use FerrisKey’s default look'
        description={`The login pages of this realm go back to FerrisKey’s own look. ${active?.name ?? 'The active theme'} stays in the list and can be activated again.`}
        confirmLabel='Use the default look'
        disabled={isBusy}
        onConfirm={() => {
          setDeactivating(false)
          onDeactivate()
        }}
      />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title='Delete theme'
        description={`${deleting?.name ?? 'This theme'} is deleted for good. Export it first if you may want it back.`}
        confirmLabel='Delete theme'
        destructive
        disabled={isBusy}
        onConfirm={() => {
          if (deleting) onDelete(deleting)
          setDeleting(null)
        }}
      />
    </>
  )
}

function NewThemeDialog({
  open,
  onOpenChange,
  taken,
  onCreate,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  taken: string[]
  onCreate: (name: string) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <NewThemeForm taken={taken} onCancel={() => onOpenChange(false)} onCreate={onCreate} />
      </DialogContent>
    </Dialog>
  )
}

function NewThemeForm({
  taken,
  onCancel,
  onCreate,
}: {
  taken: string[]
  onCancel: () => void
  onCreate: (name: string) => void
}) {
  const [name, setName] = useState('')
  const problem = name === '' ? null : describeNameProblem(name)
  const clash = taken.some((entry) => entry.trim().toLowerCase() === name.trim().toLowerCase())
  const canCreate = describeNameProblem(name) === null && !clash

  return (
    <form
      className='grid gap-4'
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        if (canCreate) onCreate(name.trim())
      }}
    >
      <DialogHeader>
        <DialogTitle>New theme</DialogTitle>
        <DialogDescription>
          It starts from FerrisKey’s default look. You will tune it in the builder.
        </DialogDescription>
      </DialogHeader>
      <div className='space-y-1.5'>
        <Label htmlFor='new-theme-name'>Name</Label>
        <Input
          id='new-theme-name'
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder='Autumn campaign'
          aria-invalid={!!problem || clash}
          autoFocus
        />
        {(problem || clash) && (
          <p className='text-xs text-destructive'>
            {problem ?? 'A theme of this realm already has this name.'}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type='button' variant='outline' onClick={onCancel}>
          Cancel
        </Button>
        <Button type='submit' disabled={!canCreate}>
          Create theme
        </Button>
      </DialogFooter>
    </form>
  )
}

function ImportDialog({
  open,
  onOpenChange,
  onImport,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImport: (theme: ThemeFile) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <ImportForm onCancel={() => onOpenChange(false)} onImport={onImport} />
      </DialogContent>
    </Dialog>
  )
}

function ImportForm({
  onCancel,
  onImport,
}: {
  onCancel: () => void
  onImport: (theme: ThemeFile) => void
}) {
  const [text, setText] = useState('')
  const [problem, setProblem] = useState<string | null>(null)

  const choose = async (file: File | undefined) => {
    if (!file) return
    if (file.size > MAX_FILE_BYTES) {
      setProblem('The file is larger than 1 MiB.')
      return
    }
    setProblem(null)
    setText(await file.text())
  }

  const submit = () => {
    const result = parseImport(text)
    if (!result.ok) {
      setProblem(result.error)
      return
    }
    onImport(result.theme)
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Import a theme</DialogTitle>
        <DialogDescription>
          Choose a file exported from here or from FerrisKey’s theme builder, or paste its JSON.
          Only the design tokens are imported: FerrisKey’s page layouts are left out.
        </DialogDescription>
      </DialogHeader>
      <div className='space-y-3'>
        <div className='space-y-1.5'>
          <Label htmlFor='import-theme-file'>File</Label>
          <Input
            id='import-theme-file'
            type='file'
            accept='application/json,.json'
            onChange={(event) => void choose(event.target.files?.[0])}
          />
        </div>
        <div className='space-y-1.5'>
          <Label htmlFor='import-theme-text'>Or paste JSON</Label>
          <textarea
            id='import-theme-text'
            value={text}
            onChange={(event) => setText(event.target.value)}
            rows={8}
            spellCheck={false}
            className='border-input w-full rounded-md border bg-transparent p-3 font-mono text-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50'
          />
        </div>
        {problem && <Alert>{problem}</Alert>}
      </div>
      <DialogFooter>
        <Button variant='outline' onClick={onCancel}>
          Cancel
        </Button>
        <Button disabled={text.trim() === ''} onClick={submit}>
          Import
        </Button>
      </DialogFooter>
    </>
  )
}
