import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { Target, WhatBox } from '../../../canvas-model'
import { SERVERS } from '../../../tool-catalogue'
import { EditorShell, Row, SharedNote } from './editor-parts'
import { ToolPicker } from './tool-picker'

const TYPES: { value: Target['type']; label: string }[] = [
  { value: 'read-only', label: 'Read-only tools' },
  { value: 'server', label: 'Every tool of a server' },
  { value: 'tools', label: 'Specific tools' },
]

const ALL_SERVERS = 'all'

function withType(type: Target['type']): Target {
  switch (type) {
    case 'read-only':
      return { type, server: null }
    case 'server':
      return { type, server: SERVERS[0].id }
    case 'tools':
      return { type, tools: [] }
  }
}

function ServerSelect({
  id,
  value,
  allowAll,
  disabled,
  onChange,
}: {
  id: string
  value: string | null
  allowAll: boolean
  disabled: boolean
  onChange: (server: string | null) => void
}) {
  return (
    <Select
      value={value ?? ALL_SERVERS}
      disabled={disabled}
      onValueChange={(next) => onChange(next === ALL_SERVERS ? null : next)}
    >
      <SelectTrigger id={id} className='w-full'>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {allowAll && <SelectItem value={ALL_SERVERS}>Every server</SelectItem>}
        {SERVERS.map((server) => (
          <SelectItem key={server.id} value={server.id}>
            {server.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

export function WhatEditor({
  box,
  usage,
  readOnly,
  onChange,
  onClose,
}: {
  box: WhatBox
  usage: number
  readOnly: boolean
  onChange: (box: WhatBox) => void
  onClose: () => void
}) {
  const target = box.target

  return (
    <EditorShell title='What' subtitle='Tools this rule covers' onClose={onClose}>
      <Row label='Type' htmlFor='what-type'>
        <Select
          value={target.type}
          disabled={readOnly}
          onValueChange={(type) => onChange({ ...box, target: withType(type as Target['type']) })}
        >
          <SelectTrigger id='what-type' className='w-full'>
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
      {target.type === 'read-only' && (
        <Row label='Server' htmlFor='what-server' hint='Tools that only look and never change anything.'>
          <ServerSelect
            id='what-server'
            value={target.server}
            allowAll
            disabled={readOnly}
            onChange={(server) => onChange({ ...box, target: { type: 'read-only', server } })}
          />
        </Row>
      )}
      {target.type === 'server' && (
        <Row label='Server' htmlFor='what-server' hint='Includes tools added to this server later.'>
          <ServerSelect
            id='what-server'
            value={target.server}
            allowAll={false}
            disabled={readOnly}
            onChange={(server) => onChange({ ...box, target: { type: 'server', server: server ?? '' } })}
          />
        </Row>
      )}
      {target.type === 'tools' && (
        <ToolPicker
          tools={target.tools}
          disabled={readOnly}
          onChange={(tools) => onChange({ ...box, target: { type: 'tools', tools } })}
        />
      )}
      <SharedNote count={usage} what='box' />
    </EditorShell>
  )
}
