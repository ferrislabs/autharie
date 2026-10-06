import { StatusBadge } from '@/components/ui/status-badge'
import type { DeploymentKind } from '../../../types/deployment'
import { ENGINES } from '../../../pricing/plans'
import { OptionCard } from './option-card'

interface Props {
  engine: DeploymentKind
  onSelect: (engine: DeploymentKind) => void
}

export function EnginePicker({ engine, onSelect }: Props) {
  return (
    <div className='grid gap-3 sm:grid-cols-2'>
      {(Object.keys(ENGINES) as DeploymentKind[]).map((value) => (
        <OptionCard
          key={value}
          selected={engine === value}
          onSelect={() => onSelect(value)}
          label={ENGINES[value].name}
          description={ENGINES[value].description}
          footer={
            <StatusBadge tone={value === 'ferriskey' ? 'accent' : 'warning'} dot={false}>
              {ENGINES[value].badge}
            </StatusBadge>
          }
        />
      ))}
    </div>
  )
}
