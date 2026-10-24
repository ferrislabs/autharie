import { Outlet, useParams } from '@tanstack/react-router'
import { Boxes } from 'lucide-react'
import { useGetDeployment } from '@/api/deployment.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { deploymentTabs } from './deployment-tabs'
import { NavTabs } from './nav-tabs'
import { SpaceSwitch } from './space-switch'
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

  const tabs = deploymentTabs({ can, deploymentPath })

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
        <TopBar crumbs={crumbs} spaces={<SpaceSwitch active='infrastructure' />} />
        <NavTabs tabs={tabs} />
      </header>
      <main>
        <Outlet />
      </main>
    </div>
  )
}
