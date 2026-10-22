import { providerById } from '../../providers'
import type { Provider } from '../../types/cloud-provider'

export function ProviderLogoTile({ provider }: { provider: Provider }) {
  const Logo = providerById(provider).icon

  return (
    <span className='flex size-10 shrink-0 items-center justify-center rounded-lg border bg-background'>
      <Logo className='size-6' />
    </span>
  )
}

interface Props {
  provider: Provider
  onSelect: (provider: Provider) => void
}

export function ProviderCard({ provider, onSelect }: Props) {
  const definition = providerById(provider)

  return (
    <button
      type='button'
      onClick={() => onSelect(provider)}
      className='flex flex-col items-start gap-3 rounded-lg border p-4 text-left transition-colors hover:border-primary/50 hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring'
    >
      <span className='flex items-center gap-3'>
        <ProviderLogoTile provider={provider} />
        <span className='font-medium'>{definition.label}</span>
      </span>
      <span className='text-sm text-muted-foreground'>{definition.description}</span>
    </button>
  )
}
