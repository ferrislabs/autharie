import type { ComponentType, SVGProps } from 'react'
import { ScalewayLogo } from './pages/ui/provider-logos'
import type { Provider } from './types/cloud-provider'

export interface CredentialField {
  id: string
  label: string
  placeholder?: string
  masked?: boolean
}

export interface ProviderDefinition {
  id: Provider
  label: string
  icon: ComponentType<SVGProps<SVGSVGElement>>
  fields: CredentialField[]
  secretKeys: string[]
  permissionSets: string[]
}

export const PROVIDERS: ProviderDefinition[] = [
  {
    id: 'scaleway',
    label: 'Scaleway',
    icon: ScalewayLogo,
    fields: [
      { id: 'name', label: 'Name', placeholder: 'Scaleway production' },
      { id: 'access_key', label: 'Access key', placeholder: 'SCWXXXXXXXXXXXXXXXXX' },
      { id: 'secret_key', label: 'Secret access key', masked: true },
      { id: 'organization_id', label: 'Organization id' },
      { id: 'project_id', label: 'Project id' },
    ],
    secretKeys: ['access_key', 'secret_key', 'project_id', 'organization_id'],
    permissionSets: [
      'KubernetesFullAccess',
      'InstancesReadOnly',
      'PrivateNetworksFullAccess',
      'ProjectReadOnly',
      'IAMReadOnly',
    ],
  },
]

export function providerById(id: Provider): ProviderDefinition {
  return PROVIDERS.find((provider) => provider.id === id) ?? PROVIDERS[0]
}
