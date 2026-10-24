import { DatabaseBackup, LayoutGrid, Radar, Settings, ShieldCheck } from 'lucide-react'
import { CAN } from '@/domain/organisations/permissions'
import type { Tab } from './nav-tabs'

interface Options {
  can: (bit: number) => boolean
  deploymentPath: (path?: string) => string
}

/**
 * The Infrastructure tabs of a deployment: running it, not what its users
 * live with. Branding, Webhooks and Agents are in the Identity space.
 */
export function deploymentTabs({ can, deploymentPath }: Options): Tab[] {
  return [
    { label: 'Overview', to: deploymentPath(), icon: LayoutGrid, exact: true },
    // Logs, traces and usage live behind their own side nav -- see
    // DeploymentObservabilityLayout -- so this section grows without
    // growing this bar.
    { label: 'Observability', to: deploymentPath('/observability'), icon: Radar },
    ...(can(CAN.viewBackups)
      ? [{ label: 'Backups', to: deploymentPath('/backups'), icon: DatabaseBackup }]
      : []),
    { label: 'Security', to: deploymentPath('/security'), icon: ShieldCheck },
    { label: 'Settings', to: deploymentPath('/settings'), icon: Settings },
  ]
}
