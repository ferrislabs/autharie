import { useEffect } from 'react'
import { useGetBrandingRealms, useGetSmtp, useRemoveSmtp, useSaveSmtp } from '@/api/smtp.api'
import { useIdentityRealm } from '@/domain/identity/use-identity-realm'
import { useMyPermissions } from '@/domain/organisations/hooks/use-my-permissions'
import { CAN } from '@/domain/organisations/permissions'
import { toRequest, type SmtpForm } from '../../smtp-form'
import { isNotFerriskey } from '../../smtp-state'
import { useBrandingScope } from '../../use-branding-scope'
import { PageEmailProvider } from '../ui/page-email-provider'

export default function PageEmailProviderFeature() {
  const scope = useBrandingScope()
  const realm = useIdentityRealm()
  const { can } = useMyPermissions()

  const smtp = useGetSmtp(scope, realm)
  const realms = useGetBrandingRealms(scope)
  const save = useSaveSmtp()
  const remove = useRemoveSmtp()

  const stored = smtp.data?.data ?? null
  const unsupported = isNotFerriskey(smtp.error) || isNotFerriskey(realms.error)
  const path =
    scope.organisationId && scope.deploymentId
      ? { organisation_id: scope.organisationId, deployment_id: scope.deploymentId, realm }
      : null

  const onSave = (form: SmtpForm) => {
    const body = toRequest(form, stored?.password_set ?? false)
    if (!path || !body || save.isPending) return

    save.mutate({ path, body })
  }

  const onRemove = () => {
    if (!path || remove.isPending) return

    remove.mutate({ path })
  }

  // The realm is in the address, so choosing another one is a navigation and
  // this page stays mounted: what a save or a removal said about the last
  // realm must not be read as the answer for the next.
  const { reset: resetSave } = save
  const { reset: resetRemove } = remove
  useEffect(() => {
    resetSave()
    resetRemove()
  }, [realm, resetSave, resetRemove])

  return (
    <PageEmailProvider
      isLoading={smtp.isLoading}
      unsupported={unsupported}
      error={smtp.error && !unsupported ? smtp.error.message : undefined}
      realm={realm}
      smtp={stored}
      canManage={can(CAN.manageInstances)}
      isSaving={save.isPending}
      isRemoving={remove.isPending}
      saveError={save.error?.message}
      removeError={remove.error?.message}
      onSave={onSave}
      onRemove={onRemove}
    />
  )
}
