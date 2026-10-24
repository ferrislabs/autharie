export interface ApplicationSummary {
  id: string
  client_id: string
  name: string
  enabled: boolean
  client_type: string
  public_client: boolean
  service_account_enabled: boolean
  protocol: string
  registration_source: string
  created_at: string
}

export interface ApplicationDetail extends ApplicationSummary {
  direct_access_grants_enabled: boolean
  require_pkce: boolean
  token_exchange_enabled: boolean
  consent_required: boolean
  redirect_uris: string[]
  access_token_lifetime?: number
  refresh_token_lifetime?: number
  id_token_lifetime?: number
  updated_at: string
}

export interface ApplicationList {
  items: ApplicationSummary[]
  next_page?: number
}
