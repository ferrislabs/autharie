import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'
import type { CloudCredential, Provider } from './types/cloud-provider'

export const SCALEWAY_SECRET_HELP =
  'Paste the JSON {"access_key": "...", "secret_key": "...", "project_id": "..."} of a Scaleway API key. It is sent once and never shown again.'

const SCALEWAY_SECRET_KEYS = ['access_key', 'secret_key', 'project_id'] as const

export interface CredentialDraft {
  label: string
  provider: Provider
  secret: string
}

export const EMPTY_CREDENTIAL_DRAFT: CredentialDraft = { label: '', provider: 'scaleway', secret: '' }

export function secretProblem(secret: string): string | null {
  if (secret.trim() === '') return null

  let parsed: unknown
  try {
    parsed = JSON.parse(secret)
  } catch {
    return 'This is not valid JSON.'
  }

  if (typeof parsed !== 'object' || parsed === null) return 'This is not a JSON object.'

  const record = parsed as Record<string, unknown>
  const missing = SCALEWAY_SECRET_KEYS.filter(
    (key) => typeof record[key] !== 'string' || record[key] === '',
  )

  return missing.length === 0 ? null : `Missing: ${missing.join(', ')}.`
}

export function canRegister(draft: CredentialDraft): boolean {
  return (
    draft.label.trim() !== '' && draft.secret.trim() !== '' && secretProblem(draft.secret) === null
  )
}

export function toRegisterRequest(draft: CredentialDraft): Schemas.RegisterCloudCredentialRequest {
  return { label: draft.label.trim(), provider: draft.provider, secret: draft.secret }
}

export function afterSubmit(): CredentialDraft {
  return EMPTY_CREDENTIAL_DRAFT
}

export const CREDENTIAL_IN_USE =
  'A deployment still runs on this account. Delete the deployment first, then the account.'

export function deletionRefusal(error: unknown): string | undefined {
  if (error instanceof ApiRequestError && error.status === 409) {
    return error.message.startsWith('HTTP ') ? CREDENTIAL_IN_USE : error.message
  }
  return error instanceof Error ? error.message : undefined
}

export function byLabel(credentials: CloudCredential[]): CloudCredential[] {
  return [...credentials].sort((a, b) => a.label.localeCompare(b.label))
}
