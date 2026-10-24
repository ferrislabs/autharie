import { Outlet } from '@tanstack/react-router'
import { useRealmPath } from '@/domain/identity/use-identity-realm'
import { brandingEntries } from './branding-entries'
import { SideNavLayout } from './side-nav'

/**
 * What an end user sees of a realm: the login pages and the emails.
 * Same shape as `DeploymentObservabilityLayout`, a side nav beside the page.
 */
export function DeploymentBrandingLayout() {
  const realmPath = useRealmPath()

  return (
    <SideNavLayout entries={brandingEntries(realmPath)}>
      <Outlet />
    </SideNavLayout>
  )
}
