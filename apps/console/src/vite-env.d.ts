interface ImportMetaEnv {
  readonly VITE_API_URL: string
  readonly VITE_OIDC_ISSUER_URL: string
  readonly VITE_OIDC_CLIENT_ID: string
  readonly VITE_SUPPORT_URL?: string
  readonly VITE_DOCS_URL?: string
  // add more environment variables as needed
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}

declare module '*.css'

declare module '*?raw' {
  const content: string
  export default content
}
