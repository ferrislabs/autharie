import { DEFAULT_REALM } from '@/lib/paths'

type Reader = Pick<Storage, 'getItem'>
type Writer = Pick<Storage, 'setItem'>

const keyFor = (deploymentId: string) => `identity.realm.${deploymentId}`

/** Null where there is no session storage, or the browser refuses it. */
function sessionStore(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.sessionStorage
  } catch {
    return null
  }
}

/**
 * The realm last looked at in a deployment.
 *
 * Webhooks and Agents sit in the Identity space without being in a realm, so
 * the tabs that do need one have nothing in the address to read. Coming back
 * to the realm you were in is what somebody expects; `master` is only where
 * you land when there has been none.
 */
export function readRealm(deploymentId: string, storage: Reader | null = sessionStore()): string {
  return storage?.getItem(keyFor(deploymentId)) || DEFAULT_REALM
}

export function rememberRealm(
  deploymentId: string,
  realm: string,
  storage: Writer | null = sessionStore(),
): void {
  storage?.setItem(keyFor(deploymentId), realm)
}
