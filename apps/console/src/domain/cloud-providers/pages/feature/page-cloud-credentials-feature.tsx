import { useState } from 'react'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { deletionRefusal } from '../../credentials'
import {
  useDeleteCloudCredential,
  useGetCloudCredentials,
  useRegisterCloudCredential,
} from '../../hooks/use-cloud-credentials'
import { PageCloudCredentials } from '../ui/page-cloud-credentials'

export default function PageCloudCredentialsFeature() {
  const organisationId = useResolvedOrganisationId()
  const credentials = useGetCloudCredentials()
  const register = useRegisterCloudCredential()
  const remove = useDeleteCloudCredential()
  const [registerRefusal, setRegisterRefusal] = useState<string>()

  return (
    <PageCloudCredentials
      credentials={credentials.data ?? []}
      isLoading={credentials.isLoading}
      isSaving={register.isPending || remove.isPending}
      registerRefusal={registerRefusal}
      deleteRefusal={deletionRefusal(remove.error)}
      onRegister={(request, onRegistered) => {
        if (!organisationId || register.isPending) return
        remove.reset()
        setRegisterRefusal(undefined)
        register.mutate(
          {
            path: { organisation_id: organisationId },
            body: request,
          },
          {
            onSuccess: onRegistered,
            onError: (error) => setRegisterRefusal(error.message),
            onSettled: () => register.reset(),
          }
        )
      }}
      onDelete={(credential) => {
        if (!organisationId || remove.isPending) return
        remove.mutate({
          path: { organisation_id: organisationId, credential_id: credential.id },
        })
      }}
      onDialogClosed={() => setRegisterRefusal(undefined)}
    />
  )
}
