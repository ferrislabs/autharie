import { useState } from 'react'
import { Cloud, Trash2 } from 'lucide-react'
import { formatDistanceToNow } from 'date-fns'
import { EmptyState, Page, PageTitle } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { byLabel } from '../../credentials'
import type { CloudCredential } from '../../types/cloud-provider'
import type { Schemas } from '@/api/api.client'
import type { Provider } from '../../types/cloud-provider'
import { CreateCredentialDialog } from './create-credential-dialog'
import { NewCredentialMenu } from './new-credential-menu'

interface Props {
  credentials: CloudCredential[]
  isLoading: boolean
  isSaving: boolean
  registerRefusal?: string
  deleteRefusal?: string
  onRegister: (request: Schemas.RegisterCloudCredentialRequest, onRegistered: () => void) => void
  onDelete: (credential: CloudCredential) => void
  onDialogClosed: () => void
}

export function PageCloudCredentials({
  credentials,
  isLoading,
  isSaving,
  registerRefusal,
  deleteRefusal,
  onRegister,
  onDelete,
  onDialogClosed,
}: Props) {
  const [adding, setAdding] = useState<Provider | null>(null)

  if (isLoading) {
    return (
      <Page>
        <Skeleton className='h-64 w-full' />
      </Page>
    )
  }

  const add = <NewCredentialMenu disabled={isSaving} onSelect={setAdding} />

  return (
    <Page>
      <PageTitle title='Cloud accounts' actions={add} />

      <p className='mt-4 text-sm text-muted-foreground'>
        The cloud provider accounts your clusters are created in. The infrastructure is billed to
        you by the provider.
      </p>

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
          <div className='overflow-x-auto rounded-lg border'>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Label</TableHead>
                  <TableHead>Provider</TableHead>
                  <TableHead>Added</TableHead>
                  <TableHead className='text-right'>
                    <span className='sr-only'>Actions</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {byLabel(credentials).map((credential) => (
                  <TableRow key={credential.id}>
                    <TableCell className='font-medium'>{credential.label}</TableCell>
                    <TableCell className='capitalize text-muted-foreground'>
                      {credential.provider}
                    </TableCell>
                    <TableCell className='text-xs text-muted-foreground'>
                      {formatDistanceToNow(new Date(credential.created_at))} ago
                    </TableCell>
                    <TableCell className='text-right'>
                      <Button
                        variant='ghost'
                        size='sm'
                        disabled={isSaving}
                        aria-label={`Delete ${credential.label}`}
                        onClick={() => onDelete(credential)}
                      >
                        <Trash2 className='h-4 w-4' />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      <CreateCredentialDialog
        provider={adding}
        onClose={() => {
          setAdding(null)
          onDialogClosed()
        }}
        isSaving={isSaving}
        refusal={registerRefusal}
        onSubmit={(request) => onRegister(request, () => setAdding(null))}
      />
    </Page>
  )
}
