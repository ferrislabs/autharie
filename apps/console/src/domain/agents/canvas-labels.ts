import type { Subject, Target } from './canvas-model'
import { summariseHours, summariseWeekdays } from './summary'
import { findServer, findTool, isUnknownServer, nameOfTool, type Catalogue } from './tool-catalogue'
import type { When } from './types'

export const PREVIEW_ABOVE = 3

export function serverLabel(catalogue: Catalogue, id: string): string {
  const label = findServer(catalogue, id)?.label ?? id
  return isUnknownServer(catalogue, id) ? `${label} (unknown server)` : label
}

export function subjectLabel(subject: Subject): string {
  if (subject.type === 'any') return 'Any user'
  if (subject.value.trim().length > 0) return subject.value
  return subject.type === 'agent' ? 'Pick an agent' : `Pick a ${subject.type}`
}

export function subjectCaption(subject: Subject): string {
  switch (subject.type) {
    case 'any':
      return 'everyone'
    case 'agent':
      return 'acts on its own'
    default:
      return subject.type
  }
}

export function targetLabel(target: Target, catalogue: Catalogue): string {
  switch (target.type) {
    case 'read-only':
      return target.server === null
        ? 'Read-only tools'
        : `Read-only on ${serverLabel(catalogue, target.server)}`
    case 'server':
      return target.server.length === 0 ? 'Pick a server' : serverLabel(catalogue, target.server)
    case 'tools':
      if (target.tools.length === 0) return 'No tools yet'
      return target.tools.length === 1 ? nameOfTool(target.tools[0]) : `${target.tools.length} tools`
  }
}

export function targetCaption(target: Target): string {
  switch (target.type) {
    case 'read-only':
      return target.server === null ? 'every server' : 'look but never change'
    case 'server':
      return 'every tool of the server'
    case 'tools': {
      const names = target.tools.map(nameOfTool)
      if (names.length <= PREVIEW_ABOVE) return names.join(', ')
      return `${names.slice(0, 3).join(', ')} +${names.length - 3}`
    }
  }
}

export function destructiveCount(target: Target, catalogue: Catalogue): number {
  if (target.type !== 'tools') return 0
  return target.tools.filter((tool) => findTool(catalogue, tool)?.destructive ?? false).length
}

export function isDestructiveTarget(target: Target, catalogue: Catalogue): boolean {
  return (
    target.type === 'tools' &&
    target.tools.length > 0 &&
    destructiveCount(target, catalogue) === target.tools.length
  )
}

export function unknownServers(target: Target, catalogue: Catalogue): string[] {
  const named =
    target.type === 'tools'
      ? target.tools.map((tool) => tool.slice(0, Math.max(tool.indexOf('/'), 0)))
      : target.server === null
        ? []
        : [target.server]
  return [...new Set(named)].filter((server) => isUnknownServer(catalogue, server))
}

export function conditionLabel(when: When | null): string | null {
  if (when === null) return null
  const parts: string[] = []
  if (when.weekdays.length > 0) parts.push(summariseWeekdays(when.weekdays))
  if (when.hours !== null) parts.push(summariseHours(when.hours))
  return parts.length > 0 ? parts.join(', ') : null
}
