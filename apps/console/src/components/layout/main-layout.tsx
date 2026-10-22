import { Outlet } from '@tanstack/react-router'
import { offersCloudProviders } from '@/domain/cloud-providers/availability'
import { useCloudProviders } from '@/domain/cloud-providers/hooks/use-cloud-providers'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { useOrganisationPath } from '@/domain/organisations/hooks/use-organisation-path'
import { NavTabs } from './nav-tabs'
import { organisationTabs } from './organisation-tabs'
import { TopBar } from './top-bar'

/**
 * What a customer sees.
 *
 * Data planes and the release catalogue are deliberately absent, for
 * operators too: they belong to whoever runs the installation, not to
 * whoever is inside one organisation, and mixing the two in one tab bar
 * makes every customer's console look like a control room they are locked
 * out of. They live under `/platform`.
 */
export function AppLayout() {
  const organisationPath = useOrganisationPath()
  const { can } = useMyPermissions()
  const cloudProviders = useCloudProviders()

  const tabs = organisationTabs({
    can,
    organisationPath,
    cloudAccounts: offersCloudProviders(cloudProviders.data),
  })

  return (
    <div className='min-h-svh bg-background'>
      <header className='sticky top-0 z-20 border-b bg-background'>
        <TopBar />
        <NavTabs tabs={tabs} />
      </header>
      <main>
        <Outlet />
      </main>
    </div>
  )
}
