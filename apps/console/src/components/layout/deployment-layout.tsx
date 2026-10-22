import { Outlet, useParams } from '@tanstack/react-router'
import {
  Bot,
  Boxes,
  DatabaseBackup,
  LayoutGrid,
  Palette,
  Radar,
  Settings,
  ShieldCheck,
  Webhook,
} from 'lucide-react'
import { useGetDeployment } from '@/api/deployment.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { CAN } from '@/domain/organisations/permissions'
import { NavTabs, type Tab } from './nav-tabs'
import { TopBar, type Crumb } from './top-bar'

/**
 * A deployment gets the whole bar to itself.
 *
 * Keeping the organisation's tabs here would mean the header answers a
 * question nobody is asking any more: once a deployment is open, everything
 * on screen is about that one instance, and "Overview" would mean two
 * different things one line apart.
 */
export function DeploymentLayout() {
  const { deploymentId } = useParams({ strict: false }) as { deploymentId?: string }
  const organisationPath = useOrganisationPath()
  const deploymentPath = useDeploymentPath()
  const { can } = useMyPermissions()

  const deployment = useGetDeployment(deploymentId ?? null)
  const name = deployment.data?.data.name

  const tabs: Tab[] = [
    { label: 'Overview', to: deploymentPath(), icon: LayoutGrid, exact: true },
    // Logs, traces and usage live behind their own side nav now -- see
    // DeploymentObservabilityLayout -- so this section grows without
    // growing this bar.
    { label: 'Observability', to: deploymentPath('/observability'), icon: Radar },
    ...(can(CAN.viewBackups)
      ? [{ label: 'Backups', to: deploymentPath('/backups'), icon: DatabaseBackup }]
      : []),
    { label: 'Security', to: deploymentPath('/security'), icon: ShieldCheck },
    { label: 'Customization', to: deploymentPath('/customization'), icon: Palette },
    ...(deployment.data?.data.kind === 'ferriskey'
      ? [{ label: 'Webhooks', to: deploymentPath('/webhooks'), icon: Webhook }]
      : []),
    ...(deployment.data?.data.kind === 'ferriskey' && can(CAN.manageInstances)
      ? [{ label: 'Agents', to: deploymentPath('/agents'), icon: Bot }]
      : []),
    { label: 'Settings', to: deploymentPath('/settings'), icon: Settings },
  ]

  const crumbs: Crumb[] = [
    {
      label: 'Deployments',
      to: organisationPath('/deployments'),
      icon: <Boxes className='h-3.5 w-3.5 text-muted-foreground' />,
    },
    // Named while it loads rather than left blank: an empty crumb makes the
    // header jump, and the id in the URL is not a name anybody recognises.
    { label: name ?? 'Loading' },
  ]

  return (
    <div className='min-h-svh bg-background'>
      <header className='sticky top-0 z-20 border-b bg-background'>
        <TopBar crumbs={crumbs} />
        <NavTabs tabs={tabs} />
      </header>
      <main>
        <Outlet />
      </main>
    </div>
  )
}
