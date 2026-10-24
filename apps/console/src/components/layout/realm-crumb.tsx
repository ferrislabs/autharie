import { ChevronsUpDown, Globe } from 'lucide-react'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { useGetBrandingRealms } from '@/api/smtp.api'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useBrandingScope } from '@/domain/branding/use-branding-scope'
import { swapRealm } from '@/lib/paths'

/**
 * The realm being looked at, and a way to look at another.
 *
 * Choosing one goes to the same page under that realm rather than to the
 * realm's front page: somebody comparing two realms' email providers should
 * not have to find the page again each time.
 */
export function RealmCrumb({ realm }: { realm: string }) {
  const navigate = useNavigate()
  const pathname = useRouterState({ select: (state) => state.location.pathname })
  const realms = useGetBrandingRealms(useBrandingScope())

  const listed = realms.data?.data ?? []
  const choices = listed.includes(realm) ? listed : [realm, ...listed]

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className='flex items-center gap-2 rounded-md px-1.5 py-1 text-sm font-medium hover:bg-accent'>
        <Globe className='h-4 w-4 text-primary' />
        <span className='max-w-40 truncate'>{realm}</span>
        <ChevronsUpDown className='h-3.5 w-3.5 text-muted-foreground' />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='start' className='w-56'>
        <DropdownMenuLabel className='text-xs text-muted-foreground'>Realms</DropdownMenuLabel>
        {choices.map((choice) => (
          <DropdownMenuItem
            key={choice}
            onClick={() => void navigate({ to: swapRealm(pathname, choice) })}
          >
            {choice}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
