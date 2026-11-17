import { useFeature } from '@/domain/features/hooks/use-feature'
import { AGENTS_FEATURE } from './agents-feature'

export function useAgentsFeature(organisationId: string | null): boolean {
  return useFeature(organisationId, AGENTS_FEATURE)
}
