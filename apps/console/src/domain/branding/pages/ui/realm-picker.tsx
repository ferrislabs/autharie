import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

interface Props {
  realm: string
  realms: string[]
  onChange: (realm: string) => void
}

export function RealmPicker({ realm, realms, onChange }: Props) {
  const choices = realms.includes(realm) ? realms : [realm, ...realms]

  return (
    <div className='flex items-center gap-2'>
      <Label htmlFor='branding-realm' className='text-muted-foreground'>
        Realm
      </Label>
      <Select value={realm} onValueChange={onChange}>
        <SelectTrigger id='branding-realm' className='w-44'>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {choices.map((choice) => (
            <SelectItem key={choice} value={choice}>
              {choice}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}
