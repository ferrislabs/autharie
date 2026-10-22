import { Boxes, Cloud, CreditCard, LayoutGrid, Settings, ShieldCheck, Users } from 'lucide-react'
import { CAN } from '@/domain/organisations/permissions'
import type { Tab } from './nav-tabs'

interface Options {
  can: (bit: number) => boolean
  organisationPath: (path?: string) => string
  cloudAccounts: boolean
}

/**
 * The tabs of an organisation, for somebody with these rights on this
 * installation.
 *
 * A tab leading to a page that answers 403 is worse than no tab: it invites
 * the click and then blames the person for it. The same goes for Cloud
 * accounts on an installation that cannot connect a provider, where every
 * attempt to register one is refused.
 */
export function organisationTabs({ can, organisationPath, cloudAccounts }: Options): Tab[] {
  return [
    { label: 'Overview', to: organisationPath(), icon: LayoutGrid, exact: true },
    ...(can(CAN.viewInstances)
      ? [{ label: 'Deployments', to: organisationPath('/deployments'), icon: Boxes }]
      : []),
    ...(can(CAN.viewInstances) && cloudAccounts
      ? [{ label: 'Cloud accounts', to: organisationPath('/cloud-accounts'), icon: Cloud }]
      : []),
    ...(can(CAN.viewMembers)
      ? [{ label: 'Members', to: organisationPath('/members'), icon: Users }]
      : []),
    ...(can(CAN.viewRoles)
      ? [{ label: 'Roles', to: organisationPath('/roles'), icon: ShieldCheck }]
      : []),
    ...(can(CAN.viewBilling)
      ? [{ label: 'Billing', to: organisationPath('/billing'), icon: CreditCard }]
      : []),
    ...(can(CAN.viewOrganisation)
      ? [{ label: 'Settings', to: organisationPath('/settings'), icon: Settings }]
      : []),
  ]
}
