import { Outlet } from '@tanstack/react-router'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { brandingEntries } from './branding-entries'
import { SideNavLayout } from './side-nav'

/**
 * What an end user sees of a deployment: the login pages and the emails.
 * Same shape as `DeploymentObservabilityLayout`, a side nav beside the page.
 */
export function DeploymentBrandingLayout() {
  const deploymentPath = useDeploymentPath()

  return (
    <SideNavLayout entries={brandingEntries(deploymentPath)}>
      <Outlet />
    </SideNavLayout>
  )
}
