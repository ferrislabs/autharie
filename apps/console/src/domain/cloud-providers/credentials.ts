import type { Schemas } from '@/api/api.client'
import { ApiRequestError } from '@/api/api.fetch'
import { providerById } from './providers'
import type { CloudCredential, Provider } from './types/cloud-provider'

export interface CredentialDraft {
  provider: Provider
  values: Record<string, string>
}

export function emptyDraft(provider: Provider): CredentialDraft {
  return { provider, values: {} }
}

export function canRegister(draft: CredentialDraft): boolean {
  return providerById(draft.provider).fields.every(
    (field) => (draft.values[field.id] ?? '').trim() !== ''
  )
}

export function toRegisterRequest(draft: CredentialDraft): Schemas.RegisterCloudCredentialRequest {
  const provider = providerById(draft.provider)
  const secret = Object.fromEntries(
    provider.secretKeys.map((key) => [key, (draft.values[key] ?? '').trim()])
  )
  return {
    provider: provider.id,
    label: (draft.values.name ?? '').trim(),
    secret: JSON.stringify(secret),
  }
}

export function afterSubmit(draft: CredentialDraft): CredentialDraft {
  const masked = providerById(draft.provider)
    .fields.filter((field) => field.masked)
    .map((field) => field.id)
  return {
    provider: draft.provider,
    values: Object.fromEntries(Object.entries(draft.values).filter(([id]) => !masked.includes(id))),
  }
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
