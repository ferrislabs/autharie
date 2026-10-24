import { AppWindow, Bot, Building2, KeyRound, LayoutGrid, Palette, Users, Webhook } from 'lucide-react'
import { CAN } from '@/domain/organisations/permissions'
import type { Tab } from './nav-tabs'

interface Options {
  can: (bit: number) => boolean
  deploymentPath: (path?: string) => string
  /** Webhooks and agents are FerrisKey's: another kind of deployment has none. */
  ferriskey: boolean
  /** The realm the realm pages open on. */
  realm: string
}

/**
 * The tabs of the Identity space.
 *
 * The first six belong to a realm and carry it in their address; Agents and
 * Webhooks are deployment-wide and do not. `soon` tabs are announced and not
 * built: they are not links, the same convention as the side navigation.
 */
export function identityTabs({ can, deploymentPath, ferriskey, realm }: Options): Tab[] {
  const inRealm = (path: string) =>
    deploymentPath(`/identity/realms/${encodeURIComponent(realm)}${path}`)

  return [
    { label: 'Overview', to: inRealm(''), icon: LayoutGrid, exact: true, soon: true },
    { label: 'Applications', to: inRealm('/applications'), icon: AppWindow },
    { label: 'Users', to: inRealm('/users'), icon: Users, soon: true },
    { label: 'Organisations', to: inRealm('/organisations'), icon: Building2, soon: true },
    { label: 'Roles', to: inRealm('/roles'), icon: KeyRound, soon: true },
    { label: 'Branding', to: inRealm('/branding'), icon: Palette },
    ...(ferriskey && can(CAN.manageInstances)
      ? [{ label: 'Agents', to: deploymentPath('/identity/agents'), icon: Bot }]
      : []),
    ...(ferriskey
      ? [{ label: 'Webhooks', to: deploymentPath('/identity/webhooks'), icon: Webhook }]
      : []),
  ]
}
