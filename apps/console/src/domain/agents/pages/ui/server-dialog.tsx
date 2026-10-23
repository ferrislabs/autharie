import { useState } from 'react'
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
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { slugFromName, validateName, validateServerUrl, validateSlug } from '../../server-rules'
import type { AgentServer, AgentServerChange, AgentServerCreate } from '../../types'
import { Field } from './field'

export type ServerSave =
  | { kind: 'create'; input: AgentServerCreate }
  | { kind: 'change'; id: string; input: AgentServerChange }

interface Props {
  open: boolean
  server: AgentServer | null
  taken: string[]
  isSaving: boolean
  error?: string
  onClose: () => void
  onSave: (save: ServerSave) => void
}

type FormProps = Omit<Props, 'open'>

function slugProblem(slug: string, taken: string[]): string | null {
  return validateSlug(slug) ?? (taken.includes(slug) ? 'Another server already uses this slug.' : null)
}

export function ServerForm({ server, taken, isSaving, error, onClose, onSave }: FormProps) {
  const editing = server !== null
  const [name, setName] = useState(server?.name ?? '')
  const [slug, setSlug] = useState(server?.slug ?? '')
  const [slugEdited, setSlugEdited] = useState(false)
  const [url, setUrl] = useState(server?.url ?? '')
  const [enabled, setEnabled] = useState(server?.enabled ?? true)

  const nameProblem = validateName(name)
  const urlProblem = validateServerUrl(url)
  const problem = editing ? null : slugProblem(slug, taken)
  const valid = nameProblem === null && urlProblem === null && problem === null

  const changeName = (next: string) => {
    setName(next)
    if (!editing && !slugEdited) setSlug(slugFromName(next))
  }

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!valid || isSaving) return
    if (server) {
      onSave({ kind: 'change', id: server.id, input: { name: name.trim(), url: url.trim(), enabled } })
    } else {
      onSave({ kind: 'create', input: { slug, name: name.trim(), url: url.trim(), enabled } })
    }
  }

  return (
    <form className='space-y-4' onSubmit={submit} noValidate>
      <Field label='Name' htmlFor='server-name'>
        <Input
          id='server-name'
          value={name}
          autoFocus
          aria-invalid={name !== '' && nameProblem !== null}
          onChange={(event) => changeName(event.target.value)}
        />
        {name !== '' && nameProblem && <p className='text-xs text-destructive'>{nameProblem}</p>}
      </Field>

      <Field
        label='Slug'
        htmlFor='server-slug'
        hint={
          editing
            ? 'The slug never changes: policies and the gateway path use it.'
            : 'Prefixes its tools (slug/tool) and its path (/mcp/slug).'
        }
      >
        <Input
          id='server-slug'
          value={slug}
          spellCheck={false}
          disabled={editing}
          className='font-mono'
          aria-invalid={slug !== '' && problem !== null}
          onChange={(event) => {
            setSlug(event.target.value)
            setSlugEdited(true)
          }}
        />
        {slug !== '' && problem && <p className='text-xs text-destructive'>{problem}</p>}
      </Field>

      <Field
        label='URL'
        htmlFor='server-url'
        hint='A public https:// address. The gateway reaches it with a token exchanged for this server, never with the agent’s own.'
      >
        <Input
          id='server-url'
          type='url'
          value={url}
          spellCheck={false}
          placeholder='https://tools.example.com/mcp'
          aria-invalid={url !== '' && urlProblem !== null}
          onChange={(event) => setUrl(event.target.value)}
        />
        {url !== '' && urlProblem && <p className='text-xs text-destructive'>{urlProblem}</p>}
      </Field>

      <div className='flex items-center justify-between gap-3 rounded-md border px-3 py-2'>
        <label htmlFor='server-enabled' className='text-sm'>
          Reachable by agents
        </label>
        <Switch id='server-enabled' checked={enabled} onCheckedChange={setEnabled} />
      </div>

      {error && (
        <p role='alert' className='text-sm text-destructive'>
          {error}
        </p>
      )}

      <DialogFooter>
        <Button type='button' variant='outline' onClick={onClose}>
          Cancel
        </Button>
        <Button type='submit' disabled={!valid || isSaving}>
          {isSaving && <Spinner />}
          {editing ? 'Save server' : 'Add server'}
        </Button>
      </DialogFooter>
    </form>
  )
}

export function ServerDialog({ open, server, ...rest }: Props) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && rest.onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{server ? `Edit ${server.name}` : 'Add an MCP server'}</DialogTitle>
          <DialogDescription>
            Agents reach it through this deployment’s gateway, under the same policies and decision log
            as FerrisKey’s own tools.
          </DialogDescription>
        </DialogHeader>
        {open && <ServerForm key={server?.id ?? 'new'} server={server} {...rest} />}
      </DialogContent>
    </Dialog>
  )
}
