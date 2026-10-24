import type { ApplicationDetail, ApplicationList, ApplicationSummary } from '../types'

const SHAPE = 'The answer is not in the shape this console expects.'

type Fields = Record<string, unknown>

function record(value: unknown): Fields {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Error(SHAPE)
  return value as Fields
}

function text(fields: Fields, name: string): string {
  const value = fields[name]
  if (typeof value !== 'string') throw new Error(SHAPE)
  return value
}

function flag(fields: Fields, name: string): boolean {
  const value = fields[name]
  if (typeof value !== 'boolean') throw new Error(SHAPE)
  return value
}

function whole(fields: Fields, name: string): number | undefined {
  const value = fields[name]
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(SHAPE)
  return value
}

function summary(fields: Fields): ApplicationSummary {
  return {
    id: text(fields, 'id'),
    client_id: text(fields, 'client_id'),
    name: text(fields, 'name'),
    enabled: flag(fields, 'enabled'),
    client_type: text(fields, 'client_type'),
    public_client: flag(fields, 'public_client'),
    service_account_enabled: flag(fields, 'service_account_enabled'),
    protocol: text(fields, 'protocol'),
    registration_source: text(fields, 'registration_source'),
    created_at: text(fields, 'created_at'),
  }
}

export function parseApplicationList(value: unknown): ApplicationList {
  const fields = record(value)
  if (!Array.isArray(fields.items)) throw new Error(SHAPE)

  const nextPage = whole(fields, 'next_page')
  return {
    items: fields.items.map((item) => summary(record(item))),
    ...(nextPage === undefined ? {} : { next_page: nextPage }),
  }
}

export function parseApplicationDetail(value: unknown): ApplicationDetail {
  const fields = record(value)
  if (
    !Array.isArray(fields.redirect_uris) ||
    !fields.redirect_uris.every((uri) => typeof uri === 'string')
  ) {
    throw new Error(SHAPE)
  }

  const lifetimes = {
    access_token_lifetime: whole(fields, 'access_token_lifetime'),
    refresh_token_lifetime: whole(fields, 'refresh_token_lifetime'),
    id_token_lifetime: whole(fields, 'id_token_lifetime'),
  }

  return {
    ...summary(fields),
    direct_access_grants_enabled: flag(fields, 'direct_access_grants_enabled'),
    require_pkce: flag(fields, 'require_pkce'),
    token_exchange_enabled: flag(fields, 'token_exchange_enabled'),
    consent_required: flag(fields, 'consent_required'),
    redirect_uris: fields.redirect_uris as string[],
    ...Object.fromEntries(Object.entries(lifetimes).filter(([, seconds]) => seconds !== undefined)),
    updated_at: text(fields, 'updated_at'),
  }
}
