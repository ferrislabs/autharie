import type { Schemas } from '@/api/api.client'

export type DeploymentStatus = Schemas.DeploymentStatus
export type DeploymentKind = Schemas.DeploymentKind

export type Deployment = Schemas.Deployment

/** Placement intent, not what a data plane is. */

export type Environment = 'production' | 'staging' | 'development'

export const ENVIRONMENT_OPTIONS: { id: Environment; label: string; description: string }[] = [
  { id: 'development', label: 'Development', description: 'To build and test an integration.' },
  {
    id: 'staging',
    label: 'Staging',
    description: 'To check a change before it reaches production.',
  },
  { id: 'production', label: 'Production', description: 'For the users of your product.' },
]

export const KIND_LABELS: Record<DeploymentKind, string> = {
  ferriskey: 'FerrisKey',
  keycloak: 'Keycloak',
}

