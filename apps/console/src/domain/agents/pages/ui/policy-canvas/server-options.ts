import { isUnknownServer, type Catalogue } from '../../../tool-catalogue'

export interface ServerOption {
  value: string
  label: string
}

export function serverOptions(catalogue: Catalogue, current: string | null): ServerOption[] {
  const known = catalogue.servers.map((server) => ({ value: server.id, label: server.label }))
  if (current === null || !isUnknownServer(catalogue, current)) return known
  return [{ value: current, label: `${current} (unknown server)` }, ...known]
}
