import { Monitor, Moon, Sun } from 'lucide-react'
import {
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from '@/components/ui/dropdown-menu'
import { useTheme } from '@/components/theme-provider'
import { THEME_OPTIONS, type ThemeChoice } from '@/components/theme-options'

const ICONS = { light: Sun, dark: Moon, system: Monitor } as const

export function ThemeMenu() {
  const { theme, setTheme } = useTheme()

  return (
    <>
      <DropdownMenuLabel className='text-xs font-normal text-muted-foreground'>
        Theme
      </DropdownMenuLabel>
      <DropdownMenuRadioGroup
        value={theme}
        onValueChange={(next) => setTheme(next as ThemeChoice)}
      >
        {THEME_OPTIONS.map(({ value, label }) => {
          const Icon = ICONS[value]

          return (
            <DropdownMenuRadioItem key={value} value={value}>
              <Icon className='h-3.5 w-3.5' />
              {label}
            </DropdownMenuRadioItem>
          )
        })}
      </DropdownMenuRadioGroup>
    </>
  )
}
