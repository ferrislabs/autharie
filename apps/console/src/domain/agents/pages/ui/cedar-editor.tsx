import { cn } from '@/lib/utils'

interface Props {
  id: string
  value: string
  readOnly?: boolean
  rows?: number
  onChange?: (value: string) => void
}

export function CedarEditor({ id, value, readOnly = false, rows = 14, onChange }: Props) {
  const lines = Math.max(value.split('\n').length, rows)
  const numbers = Array.from({ length: lines }, (_, index) => index + 1).join('\n')

  return (
    <div
      className={cn(
        'flex overflow-hidden rounded-md border border-input font-mono text-xs leading-5',
        readOnly && 'bg-muted/40',
      )}
    >
      <pre
        aria-hidden
        className='select-none overflow-hidden border-r bg-muted/50 px-2 py-2 text-right text-muted-foreground'
      >
        {numbers}
      </pre>
      <textarea
        id={id}
        className='min-w-0 flex-1 resize-y bg-transparent px-3 py-2 outline-none'
        style={{ whiteSpace: 'pre', overflowWrap: 'normal', overflowX: 'auto' }}
        rows={rows}
        value={value}
        readOnly={readOnly}
        spellCheck={false}
        autoCapitalize='off'
        autoCorrect='off'
        onChange={(event) => onChange?.(event.target.value)}
      />
    </div>
  )
}
