import { useState } from 'react'
import { Check, Copy, ExternalLink, Info } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import {
  instructionGroup,
  instructionText,
  type GuideLink,
  type GuideStep,
  type PermissionGroup,
} from '../../providers'
import { emphasised } from './emphasis'

export function ExternalLinkButton({ link }: { link: GuideLink }) {
  return (
    <Button variant='outline' size='sm' asChild>
      <a href={link.href} target='_blank' rel='noreferrer'>
        {link.label}
        <ExternalLink className='h-4 w-4' />
      </a>
    </Button>
  )
}

function PermissionGroupView({ group }: { group: PermissionGroup }) {
  const [copied, setCopied] = useState(false)

  return (
    <section className='space-y-2'>
      <div className='flex items-start justify-between gap-3'>
        <div className='space-y-0.5'>
          <h4 className='text-sm font-semibold'>
            {group.title} <span className='font-normal text-muted-foreground'>· {group.menu}</span>
          </h4>
          <p className='text-xs text-muted-foreground'>{group.scope}</p>
        </div>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          onClick={() => {
            void navigator.clipboard
              .writeText(group.sets.map((set) => set.name).join('\n'))
              .then(() => setCopied(true))
          }}
        >
          {copied ? <Check className='h-4 w-4' /> : <Copy className='h-4 w-4' />}
          Copy names
        </Button>
      </div>
      <ul className='grid gap-2 sm:grid-cols-2 md:grid-cols-3'>
        {group.sets.map((set) => (
          <li key={set.name} className='flex'>
            <Card className='flex w-full flex-col gap-1.5 p-3 shadow-none'>
              <span className='font-mono text-xs font-semibold'>{set.name}</span>
              <span className='text-sm text-muted-foreground'>{set.description}</span>
              <span className='mt-auto border-t pt-1.5 text-xs text-muted-foreground'>
                {set.note}
              </span>
            </Card>
          </li>
        ))}
      </ul>
    </section>
  )
}

interface Props {
  step: GuideStep
  groups: PermissionGroup[]
}

export function GuideStepView({ step, groups }: Props) {
  return (
    <section className='space-y-4'>
      <div className='space-y-1'>
        <h3 className='text-base font-semibold'>{step.heading}</h3>
        <p className='text-sm text-muted-foreground'>{step.intro}</p>
      </div>

      {step.link && <ExternalLinkButton link={step.link} />}

      <ol className='space-y-3'>
        {step.instructions.map((instruction, index) => {
          const text = instructionText(instruction)
          const group = groups[instructionGroup(instruction) ?? -1]

          return (
            <li key={text} className='flex gap-3 text-sm'>
              <span
                aria-hidden='true'
                className='flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium'
              >
                {index + 1}
              </span>
              <div className='min-w-0 flex-1 space-y-3'>
                <p className='text-muted-foreground'>{emphasised(text)}</p>
                {group && <PermissionGroupView group={group} />}
              </div>
            </li>
          )
        })}
      </ol>

      {step.tip && (
        <p className='flex gap-2 rounded-md border bg-muted/40 p-3 text-sm'>
          <Info className='mt-0.5 size-4 shrink-0 text-muted-foreground' />
          <span>{emphasised(step.tip)}</span>
        </p>
      )}
    </section>
  )
}
