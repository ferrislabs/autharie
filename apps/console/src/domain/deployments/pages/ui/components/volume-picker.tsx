import { BYOC_NOTE, volumeNotes } from '../../../pricing/plans'
import { VOLUMES, type Engine, type Hosting } from '../../../pricing/model'
import { describeVolume } from '../../../pricing/summary'
import { Label } from '@/components/ui/label'

interface Props {
  hosting: Hosting
  engine: Engine
  volume: number
  onChange: (volume: number) => void
}

const short = (value: number) => (value >= 1000 ? `${value / 1000}k` : String(value))

export function VolumePicker({ hosting, engine, volume, onChange }: Props) {
  if (hosting === 'byoc') {
    return (
      <div className='rounded-md border bg-muted/30 px-3 py-3 text-sm'>
        <p className='font-medium'>{describeVolume(hosting, volume)}</p>
        <p className='mt-1 text-muted-foreground'>{BYOC_NOTE.text}</p>
      </div>
    )
  }

  const index = Math.max(VOLUMES.indexOf(volume as (typeof VOLUMES)[number]), 0)
  const notes = volumeNotes(hosting, engine, volume)

  return (
    <div className='space-y-3 rounded-lg border p-4'>
      <div className='flex items-baseline justify-between gap-4'>
        <Label htmlFor='volume'>Active accounts</Label>
        <span className='font-semibold tabular-nums'>{describeVolume(hosting, volume)}</span>
      </div>
      <input
        id='volume'
        type='range'
        min={0}
        max={VOLUMES.length - 1}
        step={1}
        value={index}
        onChange={(event) => onChange(VOLUMES[Number(event.target.value)])}
        aria-valuetext={describeVolume(hosting, volume)}
        className='w-full accent-primary'
      />
      <div className='flex justify-between text-xs text-muted-foreground' aria-hidden='true'>
        {VOLUMES.map((value) => (
          <span key={value} className={value === volume ? 'font-semibold text-foreground' : ''}>
            {short(value)}
          </span>
        ))}
      </div>
      {notes.length > 0 && (
        <ul className='space-y-1 text-xs text-muted-foreground' aria-live='polite'>
          {notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      )}
    </div>
  )
}
