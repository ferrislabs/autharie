import { ChevronDown, Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { PROVIDERS } from '../../providers'
import type { Provider } from '../../types/cloud-provider'

interface Props {
  disabled?: boolean
  onSelect: (provider: Provider) => void
}

export function NewCredentialMenu({ disabled, onSelect }: Props) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size='sm' disabled={disabled}>
          <Plus className='h-4 w-4' />
          New credential
          <ChevronDown className='h-4 w-4' />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end'>
        <DropdownMenuLabel>Cloud provider</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {PROVIDERS.map((provider) => (
          <DropdownMenuItem key={provider.id} onSelect={() => onSelect(provider.id)}>
            <provider.icon className='h-4 w-4' />
            {provider.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
