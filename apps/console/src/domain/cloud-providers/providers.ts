import type { ComponentType, SVGProps } from 'react'
import { ScalewayLogo } from './pages/ui/provider-logos'
import type { Provider } from './types/cloud-provider'

export interface GuideLink {
  label: string
  href: string
}

export interface CredentialField {
  id: string
  label: string
  placeholder?: string
  masked?: boolean
  wide?: boolean
  hint?: string
  link?: GuideLink
}

export interface PermissionSet {
  name: string
  description: string
  note: string
}

export interface PermissionGroup {
  title: string
  menu: string
  scope: string
  sets: PermissionSet[]
}

export interface PermissionInstruction {
  text: string
  permissionGroup: number
}

export type GuideInstruction = string | PermissionInstruction

export interface GuideStep {
  title: string
  heading: string
  intro: string
  instructions: GuideInstruction[]
  link?: GuideLink
  tip?: string
}

export interface ConnectStepCopy {
  title: string
  heading: string
  intro: string
}

export interface ProviderDefinition {
  id: Provider
  label: string
  description: string
  icon: ComponentType<SVGProps<SVGSVGElement>>
  guide: GuideStep[]
  connect: ConnectStepCopy
  fields: CredentialField[]
  secretKeys: string[]
  permissionGroups: PermissionGroup[]
}

const SCALEWAY_IAM: GuideLink = {
  label: 'Open IAM in the Scaleway console',
  href: 'https://console.scaleway.com/iam/users',
}

export const PROVIDERS: ProviderDefinition[] = [
  {
    id: 'scaleway',
    label: 'Scaleway',
    description: 'Run your clusters in your own Scaleway project.',
    icon: ScalewayLogo,
    guide: [
      {
        title: 'Application',
        heading: 'Create an application',
        intro:
          'Autharie needs an access of its own to your Scaleway account. An IAM application is a user that is not a person: its API keys keep working when a member leaves your Organization.',
        instructions: [
          'In the Scaleway console, click **IAM & API keys** in the top-right menu.',
          'Open the **Applications** tab and click **Create application**.',
          'Enter a name, for example "autharie". The tag and the description are optional.',
          'Do not attach a policy yet: you create it in the next step.',
        ],
        link: {
          label: 'Create an application in Scaleway',
          href: 'https://console.scaleway.com/iam/applications/create',
        },
        tip: 'Until a policy is attached, an application has no permission at all.',
      },
      {
        title: 'Permissions',
        heading: 'Grant the permissions',
        intro:
          'A policy gives the application the permission sets Autharie needs. Create one policy with two rules.',
        instructions: [
          'In IAM, open the **Policies** tab and click **Create policy**.',
          'Enter a name and select your application as the **principal**.',
          {
            text: 'Add a first rule. Under **Resource management**, select these permission sets. For the scope, select **Specific projects** and choose the project your clusters are created in.',
            permissionGroup: 0,
          },
          {
            text: 'Click **Add new rule**. Under **Organization management**, select these permission sets.',
            permissionGroup: 1,
          },
          'Click **Create policy**.',
        ],
        link: SCALEWAY_IAM,
        tip: 'Do not grant any other permission: Autharie refuses a key that holds more than needed.',
      },
      {
        title: 'API key',
        heading: 'Generate an API key',
        intro: 'The key carries the permissions of the application you just gave a policy to.',
        instructions: [
          'In IAM, open the **API keys** tab and click **Generate API key**.',
          'Select your application as the **bearer**.',
          'Add a description if you want. Choose the expiration: an expired key stops working.',
          'When asked about Object Storage, choose **No, skip for now**: Autharie does not use it.',
          'Click **Generate API key**, then copy the access key and the secret key.',
        ],
        link: SCALEWAY_IAM,
        tip: 'The secret key is shown only once. Keep that window open until you have connected the account.',
      },
    ],
    connect: {
      title: 'Connect',
      heading: 'Connect your account',
      intro:
        'Paste what you copied. Autharie checks the key when you connect: if a permission is missing, or if the key holds more than needed, it tells you which.',
    },
    fields: [
      {
        id: 'name',
        label: 'Name',
        placeholder: 'Scaleway production',
        wide: true,
        hint: 'A name to recognise this account in Autharie.',
      },
      {
        id: 'access_key',
        label: 'Access key',
        placeholder: 'SCWXXXXXXXXXXXXXXXXX',
        hint: 'Shown when you generate the API key.',
      },
      {
        id: 'secret_key',
        label: 'Secret access key',
        masked: true,
        hint: 'Shown once, when you generate the API key.',
      },
      {
        id: 'organization_id',
        label: 'Organization id',
        hint: 'On the Organization settings page, click **Copy ID** next to the Organization name.',
        link: {
          label: 'Open Organization settings',
          href: 'https://console.scaleway.com/settings/organization/information',
        },
      },
      {
        id: 'project_id',
        label: 'Project id',
        hint: 'The project you chose in the policy. The default project has the same id as your Organization.',
        link: {
          label: 'Open Project settings',
          href: 'https://console.scaleway.com/project/settings',
        },
      },
    ],
    secretKeys: ['access_key', 'secret_key', 'project_id', 'organization_id'],
    permissionGroups: [
      {
        title: 'Rule 1',
        menu: 'Resource management',
        scope: 'Specific projects: the project your clusters are created in.',
        sets: [
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
        ],
      },
      {
        title: 'Rule 2',
        menu: 'Organization management',
        scope: 'These permission sets apply to the whole Organization.',
        sets: [
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
    ],
  },
]

export function providerById(id: Provider): ProviderDefinition {
  return PROVIDERS.find((provider) => provider.id === id) ?? PROVIDERS[0]
}

export function permissionSetsOf(provider: ProviderDefinition): PermissionSet[] {
  return provider.permissionGroups.flatMap((group) => group.sets)
}

export function instructionText(instruction: GuideInstruction): string {
  return typeof instruction === 'string' ? instruction : instruction.text
}

export function instructionGroup(instruction: GuideInstruction): number | undefined {
  return typeof instruction === 'string' ? undefined : instruction.permissionGroup
}
