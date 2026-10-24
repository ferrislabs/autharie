import { Outlet, useParams } from '@tanstack/react-router'
import { Boxes } from 'lucide-react'
import { useGetDeployment } from '@/api/deployment.api'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useIdentityRealm, useRememberRealm } from '@/domain/identity/use-identity-realm'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { identityTabs } from './identity-tabs'
import { NavTabs } from './nav-tabs'
import { RealmCrumb } from './realm-crumb'
import { SpaceSwitch } from './space-switch'
import { TopBar, type Crumb } from './top-bar'

/**
 * What a deployment's users live with: realms, applications, branding,
 * webhooks, agents.
 *
 * A sibling of `DeploymentLayout`, for the same reason that one is a sibling
 * of the organisation's: nested, the page would open with two headers.
 */
export function IdentityLayout() {
  const { deploymentId, realm: realmInPath } = useParams({ strict: false }) as {
    deploymentId?: string
    realm?: string
  }
  const organisationPath = useOrganisationPath()
  const deploymentPath = useDeploymentPath()
  const { can } = useMyPermissions()
  const realm = useIdentityRealm()
  useRememberRealm()

  const deployment = useGetDeployment(deploymentId ?? null)
  const name = deployment.data?.data.name

  const tabs = identityTabs({
    can,
    deploymentPath,
    realm,
    ferriskey: deployment.data?.data.kind === 'ferriskey',
  })

  const crumbs: Crumb[] = [
    {
      label: 'Deployments',
      to: organisationPath('/deployments'),
      icon: <Boxes className='h-3.5 w-3.5 text-muted-foreground' />,
    },
    { label: name ?? 'Loading', to: deploymentPath() },
    // Only where a realm is what the page is about: Agents and Webhooks are
    // the deployment's, and a realm in the header would read as a filter.
    ...(realmInPath ? [{ label: realmInPath, node: <RealmCrumb realm={realmInPath} /> }] : []),
  ]

  return (
    <div className='min-h-svh bg-background'>
      <header className='sticky top-0 z-20 border-b bg-background'>
        <TopBar crumbs={crumbs} spaces={<SpaceSwitch active='identity' />} />
        <NavTabs tabs={tabs} />
      </header>
      <main>
        <Outlet />
      </main>
    </div>
  )
}
