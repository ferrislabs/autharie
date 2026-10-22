import type { ComponentType, SVGProps } from 'react'
import { ScalewayLogo } from './pages/ui/provider-logos'
import type { Provider } from './types/cloud-provider'

export interface CredentialField {
  id: string
  label: string
  placeholder?: string
  masked?: boolean
  wide?: boolean
}

export interface PermissionSet {
  name: string
  description: string
  note: string
}

export interface ProviderDefinition {
  id: Provider
  label: string
  description: string
  icon: ComponentType<SVGProps<SVGSVGElement>>
  fields: CredentialField[]
  secretKeys: string[]
  permissionSets: PermissionSet[]
}

export const PROVIDERS: ProviderDefinition[] = [
  {
    id: 'scaleway',
    label: 'Scaleway',
    description: 'Run your clusters in your own Scaleway project.',
    icon: ScalewayLogo,
    fields: [
      { id: 'name', label: 'Name', placeholder: 'Scaleway production', wide: true },
      { id: 'access_key', label: 'Access key', placeholder: 'SCWXXXXXXXXXXXXXXXXX' },
      { id: 'secret_key', label: 'Secret access key', masked: true },
      { id: 'organization_id', label: 'Organization id' },
      { id: 'project_id', label: 'Project id' },
    ],
    secretKeys: ['access_key', 'secret_key', 'project_id', 'organization_id'],
    permissionSets: [
      {
        name: 'KubernetesFullAccess',
        description:
          'Autharie creates the Kubernetes cluster and its node pool in your project, resizes them, and deletes them.',
        note: 'Full access: read and write on Kubernetes.',
      },
      {
        name: 'InstancesReadOnly',
        description:
          'Autharie reads the catalogue of server types, to offer node sizes and to estimate the cost of your cluster.',
        note: 'Read only: nothing is created or changed with it.',
      },
      {
        name: 'PrivateNetworksFullAccess',
        description:
          'Autharie creates a private network for the cluster, and deletes it with the cluster.',
        note: 'Full access: read and write on private networks.',
      },
      {
        name: 'ProjectReadOnly',
        description:
          'Autharie reads your project, to check that it belongs to the organization id you give.',
        note: 'Read only: nothing is created or changed with it.',
      },
      {
        name: 'IAMReadOnly',
        description:
          'Autharie reads the policies of the key, to check that it holds the permissions above and nothing more.',
        note: 'Read only, and only used for that check.',
      },
    ],
  },
]

export function providerById(id: Provider): ProviderDefinition {
  return PROVIDERS.find((provider) => provider.id === id) ?? PROVIDERS[0]
}
