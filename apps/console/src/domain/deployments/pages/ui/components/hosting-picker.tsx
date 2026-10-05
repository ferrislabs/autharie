import type { Hosting } from '../../../pricing/model'
import { OptionCard } from './option-card'

interface Props {
  hosting: Hosting
  onSelect: (hosting: Hosting) => void
  customerCloudAvailable: boolean
}

export function HostingPicker({ hosting, onSelect, customerCloudAvailable }: Props) {
  return (
    <div className='space-y-3'>
      <div className='grid gap-3 sm:grid-cols-2'>
        <OptionCard
          selected={hosting === 'managed'}
          onSelect={() => onSelect('managed')}
          label='Managed by Autharie'
          description='Runs on the platform operated for you. Priced by the number of active accounts.'
        />
        {customerCloudAvailable && (
          <OptionCard
            selected={hosting === 'byoc'}
            onSelect={() => onSelect('byoc')}
            label='In your cloud'
            description='A cluster created in your own cloud account, billed by your provider. A flat fee from Autharie, users unlimited.'
          />
        )}
      </div>
      {!customerCloudAvailable && (
        <p className='text-xs text-muted-foreground'>
          Running in your own cloud is available with FerrisKey.
        </p>
      )}
    </div>
  )
}
