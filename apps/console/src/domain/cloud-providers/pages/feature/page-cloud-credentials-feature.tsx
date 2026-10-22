import { useState } from 'react'
import { useResolvedOrganisationId } from '@/domain/organisations/hooks/use-resolved-organisation-id'
import { deletionRefusal, isCustomerCloudOff } from '../../credentials'
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
  const [unavailable, setUnavailable] = useState<string>()

  return (
    <PageCloudCredentials
      credentials={credentials.data ?? []}
      isLoading={credentials.isLoading}
      isSaving={register.isPending || remove.isPending}
      registerRefusal={registerRefusal}
      unavailable={unavailable}
      deleteRefusal={deletionRefusal(remove.error)}
      onRegister={(request, onDone) => {
        if (!organisationId || register.isPending) return
        remove.reset()
        setRegisterRefusal(undefined)
        setUnavailable(undefined)
        register.mutate(
          {
            path: { organisation_id: organisationId },
            body: request,
          },
          {
            onSuccess: onDone,
            onError: (error) => {
              if (isCustomerCloudOff(error)) {
                setUnavailable(error.message)
                onDone()
              } else {
                setRegisterRefusal(error.message)
              }
            },
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
