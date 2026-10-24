import { useParams } from '@tanstack/react-router'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { useIdentityRealm, useRealmPath } from '@/domain/identity/use-identity-realm'
import { useApplication } from '../../hooks/use-applications'
import { PageApplicationDetail } from '../ui/page-application-detail'

function ApplicationOfRealm({ realm, applicationId }: { realm: string; applicationId: string }) {
  const realmPath = useRealmPath()
  const deploymentPath = useDeploymentPath()
  const { view, retry } = useApplication(realm, applicationId)

  return (
    <PageApplicationDetail
      realm={realm}
      application={view.phase === 'ready' ? view.result : null}
      isLoading={view.phase === 'loading'}
      isSlow={view.phase === 'loading' && view.slow}
      error={view.phase === 'failed' ? view.message : undefined}
      backPath={realmPath('/applications')}
      agentsPath={deploymentPath('/identity/agents')}
      onRetry={retry}
    />
  )
}

export default function PageApplicationDetailFeature() {
  const realm = useIdentityRealm()
  const { applicationId } = useParams({ strict: false }) as { applicationId?: string }

  return <ApplicationOfRealm key={`${realm}/${applicationId}`} realm={realm} applicationId={applicationId ?? ''} />
}
