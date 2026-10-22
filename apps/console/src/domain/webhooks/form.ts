import type { Schemas } from '@/api/api.client'
import { checkHeaders, headersFromNames, toHeaderRequests, type HeaderRow } from './headers'

export interface WebhookForm {
  realm: string
  name: string
  description: string
  endpoint: string
  events: string[]
  headers: HeaderRow[]
}

export type FormProblems = Partial<
  Record<'realm' | 'name' | 'endpoint' | 'events' | 'headers', string>
>

export function emptyForm(realm: string): WebhookForm {
  return { realm, name: '', description: '', endpoint: '', events: [], headers: [] }
}

export function formFromWebhook(webhook: Schemas.Webhook): WebhookForm {
  return {
    realm: webhook.realm,
    name: webhook.name,
    description: webhook.description ?? '',
    endpoint: webhook.endpoint,
    events: [...webhook.events],
    headers: headersFromNames(webhook.headers),
  }
}

export function checkEndpoint(raw: string): string | null {
  const value = raw.trim()
  if (value === '') return 'Enter the address that should receive the events.'

  let url: URL
  try {
    url = new URL(value)
  } catch {
    return 'This is not an address. Start it with https://, for example https://example.com/hooks.'
  }

  if (url.protocol !== 'https:') {
    return 'Use an https:// address. Events carry user data, so plain http is refused.'
  }
  return url.hostname === '' ? 'The address has no host name.' : null
}

export function endpointHost(endpoint: string): string {
  try {
    return new URL(endpoint).host
  } catch {
    return endpoint
  }
}

export function checkForm(form: WebhookForm): FormProblems {
  const problems: FormProblems = {}

  if (form.realm.trim() === '') problems.realm = 'Choose the realm whose events to send.'
  if (form.name.trim() === '') problems.name = 'Give the webhook a name.'
  const endpoint = checkEndpoint(form.endpoint)
  if (endpoint) problems.endpoint = endpoint
  if (form.events.length === 0) problems.events = 'Choose at least one event.'
  const headers = checkHeaders(form.headers)
  if (headers) problems.headers = headers

  return problems
}

export function isReadyToSave(form: WebhookForm): boolean {
  return Object.keys(checkForm(form)).length === 0
}

export function toRequestBody(form: WebhookForm): Schemas.WebhookRequestBody {
  const description = form.description.trim()

  return {
    realm: form.realm,
    name: form.name.trim(),
    description: description === '' ? null : description,
    endpoint: form.endpoint.trim(),
    events: form.events,
    headers: toHeaderRequests(form.headers),
  }
}
