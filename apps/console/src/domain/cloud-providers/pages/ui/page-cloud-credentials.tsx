import { useState } from 'react'
import { Cloud, Plus } from 'lucide-react'
import { EmptyState, Page, PageTitle } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { byLabel } from '../../credentials'
import type { CloudCredential } from '../../types/cloud-provider'
import type { Schemas } from '@/api/api.client'
import { ConnectProviderDialog } from './connect-provider-dialog'
import type { ConnectStep } from './connect-step'
import { CredentialCard } from './credential-card'

interface Props {
  credentials: CloudCredential[]
  isLoading: boolean
  isSaving: boolean
  registerRefusal?: string
  unavailable?: string
  deleteRefusal?: string
  onRegister: (request: Schemas.RegisterCloudCredentialRequest, onDone: () => void) => void
  onDelete: (credential: CloudCredential) => void
  onDialogClosed: () => void
}

export function PageCloudCredentials({
  credentials,
  isLoading,
  isSaving,
  registerRefusal,
  unavailable,
  deleteRefusal,
  onRegister,
  onDelete,
  onDialogClosed,
}: Props) {
  const [step, setStep] = useState<ConnectStep | null>(null)

  if (isLoading) {
    return (
      <Page>
        <Skeleton className='h-64 w-full' />
      </Page>
    )
  }

  const add = (
    <Button size='sm' disabled={isSaving} onClick={() => setStep({ provider: null })}>
      <Plus className='h-4 w-4' />
      Connect provider
    </Button>
  )

  return (
    <Page>
      <PageTitle title='Cloud accounts' actions={add} />

      <p className='mt-4 text-sm text-muted-foreground'>
        The cloud provider accounts your clusters are created in. The infrastructure is billed to
        you by the provider.
      </p>

      {unavailable && (
        <p
          role='alert'
          className='mt-4 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'
        >
          {unavailable}
        </p>
      )}

      {deleteRefusal && (
        <p
          role='alert'
          className='mt-4 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive'
        >
          {deleteRefusal}
        </p>
      )}

      <div className='mt-6'>
        {credentials.length === 0 ? (
          <EmptyState
            icon={<Cloud className='h-5 w-5' />}
            title='No cloud account yet'
            description='Add one to run a deployment on a cluster in your own cloud account.'
            action={add}
          />
        ) : (
          <div className='grid gap-4 sm:grid-cols-2 lg:grid-cols-3'>
            {byLabel(credentials).map((credential) => (
              <CredentialCard
                key={credential.id}
                credential={credential}
                disabled={isSaving}
                onDelete={onDelete}
              />
            ))}
          </div>
        )}
      </div>

      <ConnectProviderDialog
        step={step}
        onStep={setStep}
        onClose={() => {
          setStep(null)
          onDialogClosed()
        }}
        isSaving={isSaving}
        refusal={registerRefusal}
        onSubmit={(request) => onRegister(request, () => setStep(null))}
      />
    </Page>
  )
}
