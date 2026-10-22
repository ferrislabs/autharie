import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { Subject, SubjectType, WhoBox } from '../../../canvas-model'
import { EditorShell, Row, SharedNote } from './editor-parts'

const TYPES: { value: SubjectType; label: string }[] = [
  { value: 'any', label: 'Any user' },
  { value: 'group', label: 'Users in a group' },
  { value: 'role', label: 'Users with a role' },
  { value: 'user', label: 'A specific user' },
  { value: 'agent', label: 'An autonomous agent' },
]

const PLACEHOLDER: Record<Exclude<SubjectType, 'any'>, string> = {
  group: 'e.g. engineering',
  role: 'e.g. admin',
  user: 'e.g. a user id or username',
  agent: 'e.g. the agent’s client id',
}

const HINT: Record<Exclude<SubjectType, 'any'>, string> = {
  group: 'Everyone in this group.',
  role: 'Everyone holding this role.',
  user: 'Exactly this user.',
  agent: 'An agent acting on its own, with its own credentials.',
}

function withType(subject: Subject, type: SubjectType): Subject {
  if (type === 'any') return { type: 'any' }
  return { type, value: subject.type === 'any' ? '' : subject.value }
}

export function WhoEditor({
  box,
  usage,
  readOnly,
  onChange,
  onClose,
}: {
  box: WhoBox
  usage: number
  readOnly: boolean
  onChange: (box: WhoBox) => void
  onClose: () => void
}) {
  const subject = box.subject

  return (
    <EditorShell title='Who' subtitle='People this rule applies to' onClose={onClose}>
      <Row label='Type' htmlFor='who-type'>
        <Select
          value={subject.type}
          disabled={readOnly}
          onValueChange={(type) => onChange({ ...box, subject: withType(subject, type as SubjectType) })}
        >
          <SelectTrigger id='who-type' className='w-full'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {TYPES.map((entry) => (
              <SelectItem key={entry.value} value={entry.value}>
                {entry.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Row>
      {subject.type !== 'any' && (
        <Row label='Name' htmlFor='who-value' hint={HINT[subject.type]}>
          <Input
            id='who-value'
            autoFocus={!readOnly}
            value={subject.value}
            disabled={readOnly}
            placeholder={PLACEHOLDER[subject.type]}
            spellCheck={false}
            onChange={(event) => onChange({ ...box, subject: { type: subject.type, value: event.target.value } as Subject })}
          />
        </Row>
      )}
      <SharedNote count={usage} what='box' />
    </EditorShell>
  )
}
