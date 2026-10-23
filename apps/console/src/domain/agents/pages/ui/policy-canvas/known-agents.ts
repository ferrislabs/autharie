import { createContext, useContext } from 'react'
import type { KnownAgent } from '../../../registry'

export const KnownAgentsContext = createContext<KnownAgent[]>([])

export function useKnownAgents(): KnownAgent[] {
  return useContext(KnownAgentsContext)
}
