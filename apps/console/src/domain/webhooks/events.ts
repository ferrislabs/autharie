export interface EventInfo {
  name: string
  group: string
}

export interface EventGroup {
  group: string
  events: string[]
}

export type GroupSelection = 'none' | 'some' | 'all'

export function groupEvents(catalogue: EventInfo[], search = ''): EventGroup[] {
  const needle = search.trim().toLowerCase()
  const groups = new Map<string, string[]>()

  for (const { name, group } of catalogue) {
    const matches =
      needle === '' || name.toLowerCase().includes(needle) || group.toLowerCase().includes(needle)
    if (!matches) continue
    groups.set(group, [...(groups.get(group) ?? []), name])
  }

  return [...groups.entries()]
    .map(([group, events]) => ({ group, events }))
    .sort((a, b) => a.group.localeCompare(b.group))
}

export function groupSelection(group: EventGroup, selected: string[]): GroupSelection {
  const chosen = group.events.filter((name) => selected.includes(name)).length
  if (chosen === 0) return 'none'
  return chosen === group.events.length ? 'all' : 'some'
}

export function toggleGroup(group: EventGroup, selected: string[]): string[] {
  if (groupSelection(group, selected) === 'all') {
    return selected.filter((name) => !group.events.includes(name))
  }
  return [...selected, ...group.events.filter((name) => !selected.includes(name))]
}

export function toggleEvent(selected: string[], name: string): string[] {
  return selected.includes(name) ? selected.filter((item) => item !== name) : [...selected, name]
}
