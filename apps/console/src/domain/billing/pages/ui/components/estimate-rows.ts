import type { Schemas } from '@/api/api.client'

export type EstimateRow = {
  id: string
  name: string
  amount: number
}

function nameOf(id: string, names: ReadonlyMap<string, string>): string {
  return names.get(id) ?? `Unknown deployment (${id.slice(0, 8)})`
}

export function estimateRows(
  lines: ReadonlyArray<Schemas.StatementLineResponse>,
  names: ReadonlyMap<string, string>,
): EstimateRow[] {
  const amounts = new Map<string, number>()
  for (const line of lines) {
    amounts.set(line.deployment_id, (amounts.get(line.deployment_id) ?? 0) + line.amount)
  }

  return [...amounts]
    .map(([id, amount]) => ({ id, name: nameOf(id, names), amount }))
    .sort((a, b) => a.name.localeCompare(b.name))
}
