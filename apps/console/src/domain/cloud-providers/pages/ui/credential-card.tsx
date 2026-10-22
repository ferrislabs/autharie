import { formatDistanceToNow } from 'date-fns'
import { Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { providerById } from '../../providers'
import type { CloudCredential } from '../../types/cloud-provider'
import { ProviderLogoTile } from './provider-card'

interface Props {
  credential: CloudCredential
  disabled?: boolean
  onDelete: (credential: CloudCredential) => void
}

export function CredentialCard({ credential, disabled, onDelete }: Props) {
  const definition = providerById(credential.provider)

  return (
    <div className='flex flex-col gap-3 rounded-lg border bg-card p-4'>
      <div className='flex items-start justify-between gap-2'>
        <ProviderLogoTile provider={credential.provider} />
        <Button
          variant='ghost'
          size='icon'
          disabled={disabled}
          aria-label={`Delete ${credential.label}`}
          onClick={() => onDelete(credential)}
        >
          <Trash2 className='h-4 w-4' />
        </Button>
      </div>
      <div className='space-y-1'>
        <p className='font-medium'>{credential.label}</p>
        <p className='text-xs font-medium text-muted-foreground'>{definition.label}</p>
        <p className='text-sm text-muted-foreground'>{definition.description}</p>
      </div>
      <p className='mt-auto text-xs text-muted-foreground'>
        Added {formatDistanceToNow(new Date(credential.created_at))} ago
      </p>
    </div>
  )
}
