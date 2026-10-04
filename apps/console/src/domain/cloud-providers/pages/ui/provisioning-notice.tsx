import { AlertTriangle, Loader2 } from 'lucide-react'
import type { ProvisioningNotice } from '../../provisioning'

export function ProvisioningNoticeBanner({ notice }: { notice: ProvisioningNotice }) {
  if (notice.kind === 'creating') {
    return (
      <div
        role='status'
        className='flex items-start gap-3 rounded-lg border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-800 dark:border-blue-900 dark:bg-blue-950 dark:text-blue-200'
      >
        <Loader2 className='mt-0.5 h-4 w-4 shrink-0 animate-spin' />
        <div>
          <p className='font-medium'>{notice.message}</p>
          <p className='text-xs opacity-80'>
            This takes a few minutes. The deployment starts once the cluster is ready.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div
      role='alert'
      className='flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive'
    >
      <AlertTriangle className='mt-0.5 h-4 w-4 shrink-0' />
      <div>
        <p className='font-medium'>The cluster could not be created</p>
        <p className='text-xs'>{notice.message}</p>
      </div>
    </div>
  )
}
