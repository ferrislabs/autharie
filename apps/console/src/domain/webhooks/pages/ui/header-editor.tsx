import { Plus, Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { blankHeader, type HeaderRow } from '../../headers'

interface Props {
  rows: HeaderRow[]
  onChange: (rows: HeaderRow[]) => void
}

export function HeaderEditor({ rows, onChange }: Props) {
  const update = (key: string, patch: Partial<HeaderRow>) =>
    onChange(rows.map((row) => (row.key === key ? { ...row, ...patch } : row)))

  return (
    <div className='space-y-2'>
      {rows.length === 0 && (
        <p className='text-sm text-muted-foreground'>
          None. Add one if your service expects a token or an API key on each request.
        </p>
      )}

      {rows.map((row) => (
        <div key={row.key} className='flex items-center gap-2'>
          <Input
            aria-label='Header name'
            value={row.name}
            disabled={row.stored}
            onChange={(event) => update(row.key, { name: event.target.value })}
            placeholder='X-Api-Key'
            className='w-56 font-mono'
          />
          {row.stored && !row.replacing ? (
            <>
              <Input
                aria-label={`Value of ${row.name}`}
                value='••••• (unchanged)'
                disabled
                readOnly
                className='flex-1'
              />
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => update(row.key, { replacing: true, value: '' })}
              >
                Replace value
              </Button>
            </>
          ) : (
            <>
              <Input
                aria-label={`Value of ${row.name || 'the new header'}`}
                type='password'
                autoComplete='off'
                value={row.value}
                onChange={(event) => update(row.key, { value: event.target.value })}
                placeholder={row.stored ? 'New value' : 'Value'}
                className='flex-1'
              />
              {row.stored && (
                <Button
                  type='button'
                  variant='ghost'
                  size='sm'
                  onClick={() => update(row.key, { replacing: false, value: '' })}
                >
                  Keep current
                </Button>
              )}
            </>
          )}
          <Button
            type='button'
            variant='ghost'
            size='icon-sm'
            aria-label={`Remove ${row.name || 'this header'}`}
            onClick={() => onChange(rows.filter((item) => item.key !== row.key))}
          >
            <Trash2 className='h-3.5 w-3.5' />
          </Button>
        </div>
      ))}

      <Button
        type='button'
        variant='outline'
        size='sm'
        onClick={() => onChange([...rows, blankHeader(`new:${crypto.randomUUID()}`)])}
      >
        <Plus className='h-3.5 w-3.5' />
        Add header
      </Button>
    </div>
  )
}
