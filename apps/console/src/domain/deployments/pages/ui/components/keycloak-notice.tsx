import { Button } from '@/components/ui/button'
import type { Hosting } from '../../../pricing/model'
import { ferriskeyOnlyFeatures, KEYCLOAK_NOTICE } from '../../../pricing/plans'

interface Props {
  hosting: Hosting
  onSwitch: () => void
}

export function KeycloakNotice({ hosting, onSwitch }: Props) {
  return (
    <div
      role='alert'
      className='space-y-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'
    >
      <div className='flex flex-wrap items-start justify-between gap-3'>
        <div className='space-y-1'>
          <p className='font-medium'>{KEYCLOAK_NOTICE.title}</p>
          <p>{KEYCLOAK_NOTICE.warning}</p>
        </div>
        <Button type='button' size='sm' variant='outline' onClick={onSwitch}>
          {KEYCLOAK_NOTICE.switch}
        </Button>
      </div>
      <p>{KEYCLOAK_NOTICE.intro}</p>
      <ul className='space-y-1'>
        {ferriskeyOnlyFeatures(hosting).map((text) => (
          <li key={text}>× {text}</li>
        ))}
      </ul>
    </div>
  )
}
