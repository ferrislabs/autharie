import { useState } from 'react'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useIdentityRealm, useRealmPath } from '@/domain/identity/use-identity-realm'
import { useApplicationList } from '../../hooks/use-applications'
import {
  canGoPrevious,
  initialListState,
  nextPage,
  previousPage,
  submitSearch,
} from '../../list-state'
import { PageApplications } from '../ui/page-applications'

function ApplicationsOfRealm({ realm }: { realm: string }) {
  const realmPath = useRealmPath()
  const deploymentPath = useDeploymentPath()
  const [state, setState] = useState(initialListState)
  const { view, retry } = useApplicationList(realm, state)

  const list = view.phase === 'ready' ? view.result : null
  const nextPageNumber = list?.next_page

  return (
    <PageApplications
      realm={realm}
      applications={list?.items ?? null}
      isLoading={view.phase === 'loading'}
      isSlow={view.phase === 'loading' && view.slow}
      error={view.phase === 'failed' ? view.message : undefined}
      search={state.search}
      page={state.page}
      hasNext={nextPageNumber !== undefined}
      canGoPrevious={canGoPrevious(state)}
      detailPath={(id) => realmPath(`/applications/${encodeURIComponent(id)}`)}
      agentsPath={deploymentPath('/identity/agents')}
      onSearch={(term) => setState((current) => submitSearch(current, term))}
      onNext={() => setState((current) => nextPage(current, nextPageNumber))}
      onPrevious={() => setState(previousPage)}
      onRetry={retry}
    />
  )
}

export default function PageApplicationsFeature() {
  const realm = useIdentityRealm()

  return <ApplicationsOfRealm key={realm} realm={realm} />
}
