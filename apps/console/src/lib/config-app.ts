import { AppConfig, AppLinks, RuntimeConfig } from '@/types/config'
import { createUserManager } from './auth/user-manager'

/// `public/config.json` ships as a template whose values the container
/// substitutes at start. Served straight from `public/` by the dev server,
/// nothing substitutes them, and `"${OIDC_ISSUER_URL}"` is a non-empty string
/// that passes every truthiness check -- so the issuer became a relative URL
/// and authentication failed with no clue why.
export function isUnsubstituted(value: unknown): boolean {
  return typeof value === 'string' && /^\$\{[A-Z_]+\}$/.test(value.trim())
}

export function usableLink(value: unknown): string | undefined {
  if (typeof value !== 'string' || isUnsubstituted(value)) return undefined
  const link = value.trim()
  return /^(https?:\/\/|mailto:)/i.test(link) ? link : undefined
}

let links: AppLinks = {}

export function appLinks(): AppLinks {
  return links
}

async function loadConfigFromFile(): Promise<RuntimeConfig | null> {
  try {
    const response = await fetch('/config.json', {
      cache: 'no-cache',
      headers: {
        'Content-Type': 'application/json',
      },
    })

    if (!response.ok) {
      console.warn('config.json not found, falling back to environment variables')
      return null
    }

    const config = await response.json()

    if (isUnsubstituted(config.oidc_issuer_url) || isUnsubstituted(config.oidc_client_id)) {
      console.warn(
        'config.json still holds its unsubstituted placeholders, falling back to environment variables'
      )
      return null
    }

    if (!config.oidc_issuer_url || !config.oidc_client_id) {
      console.warn('config.json is missing required fields, falling back to environment variables')
      return null
    }
    console.log('Loaded configuration from config.json:', config)
    return {
      IN_DEVELOPMENT_MODE: false,
      OIDC_ISSUER_URL: config.oidc_issuer_url,
      OIDC_CLIENT_ID: config.oidc_client_id,
      API_URL: config.api_url,
      SUPPORT_URL: usableLink(config.support_url),
      DOCS_URL: usableLink(config.docs_url),
    } as RuntimeConfig
  } catch (error) {
    console.warn('Failed to load config.json:', error)
    return null
  }
}

function loadConfigFromEnv(): RuntimeConfig | null {
  const issuerUrl = import.meta.env.VITE_OIDC_ISSUER_URL
  const clientId = import.meta.env.VITE_OIDC_CLIENT_ID
  const apiUrl = import.meta.env.VITE_API_URL

  console.log('Loaded configuration from environment variables:', {
    OIDC_ISSUER_URL: issuerUrl,
    OIDC_CLIENT_ID: clientId,
    API_URL: apiUrl,
  })

  if (!issuerUrl || !clientId) {
    return null
  }

  return {
    OIDC_ISSUER_URL: issuerUrl,
    OIDC_CLIENT_ID: clientId,
    API_URL: apiUrl,
    SUPPORT_URL: usableLink(import.meta.env.VITE_SUPPORT_URL),
    DOCS_URL: usableLink(import.meta.env.VITE_DOCS_URL),
    IN_DEVELOPMENT_MODE: true,
  }
}

export async function loadAppConfig(): Promise<AppConfig> {
  let runtimeConfig = await loadConfigFromFile()

  if (!runtimeConfig) {
    console.log('Falling back to environment variables for configuration')
    runtimeConfig = loadConfigFromEnv()
  }

  // Validate configuration
  if (!runtimeConfig || !runtimeConfig.OIDC_ISSUER_URL || !runtimeConfig.OIDC_CLIENT_ID) {
    throw new Error(
      'Missing required configuration. Please ensure OIDC_ISSUER_URL and OIDC_CLIENT_ID are set in config.json or environment variables.'
    )
  }

  const config: AppConfig = {
    oidc: {
      issuer_url: runtimeConfig.OIDC_ISSUER_URL,
      client_id: runtimeConfig.OIDC_CLIENT_ID,
    },
    api_url: runtimeConfig.API_URL,
    in_development_mode: runtimeConfig.IN_DEVELOPMENT_MODE,
    support_url: runtimeConfig.SUPPORT_URL,
    docs_url: runtimeConfig.DOCS_URL,
  }

  return config
}

export function setupOidcConfiguration(config: AppConfig): void {
  window.issuerUrl = config.oidc.issuer_url
  window.inDevelopmentMode = config.in_development_mode
  links = { support_url: config.support_url, docs_url: config.docs_url }
  createUserManager(config)
}

export async function initializeAppConfig(): Promise<{
  config: AppConfig | null
  error: string | null
}> {
  try {
    const config = await loadAppConfig()
    setupOidcConfiguration(config)

    return {
      config,
      error: null,
    }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : 'Unknown configuration error'
    console.error('Failed to initialize app configuration:', errorMessage)

    return {
      config: null,
      error: errorMessage,
    }
  }
}
