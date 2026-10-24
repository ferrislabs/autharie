import { useCallback, useEffect } from 'react'
import { useParams } from '@tanstack/react-router'
import { useDeploymentPath } from '@/domain/deployments/hooks/use-deployment-path'
import { readRealm, rememberRealm } from './realm-memory'

interface Params {
  realm?: string
  deploymentId?: string
}

/**
 * The realm the Identity space is looking at: the one in the address, else the
 * one last looked at in this deployment.
 */
export function useIdentityRealm(): string {
  const { realm, deploymentId } = useParams({ strict: false }) as Params

  return realm ?? readRealm(deploymentId ?? '')
}

/** Keeps the realm in the address as the one to come back to. */
export function useRememberRealm(): void {
  const { realm, deploymentId } = useParams({ strict: false }) as Params

  useEffect(() => {
    if (realm && deploymentId) rememberRealm(deploymentId, realm)
  }, [realm, deploymentId])
}

/** Paths inside the realm that is currently open. */
export function useRealmPath() {
  const deploymentPath = useDeploymentPath()
  const realm = useIdentityRealm()

  return useCallback(
    (path = '') => deploymentPath(`/identity/realms/${encodeURIComponent(realm)}${path}`),
    [deploymentPath, realm],
  )
}
