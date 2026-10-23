import { POLICY_TEMPLATES, type PolicyTemplateDraft } from '../../policy-templates'

interface Props {
  onPick: (draft: PolicyTemplateDraft) => void
}

export function TemplateCards({ onPick }: Props) {
  return (
    <ul className='grid gap-3 sm:grid-cols-2'>
      {POLICY_TEMPLATES.map((template) => (
        <li key={template.id}>
          <button
            type='button'
            className='h-full w-full rounded-lg border bg-card p-4 text-left transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring'
            onClick={() => onPick(template.build())}
          >
            <span className='block text-sm font-medium'>{template.title}</span>
            <span className='mt-1 block text-xs text-muted-foreground'>{template.description}</span>
          </button>
        </li>
      ))}
    </ul>
  )
}
