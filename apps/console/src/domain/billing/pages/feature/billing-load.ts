export const PERIOD_UNKNOWN = 'The current month could not be determined.'

export function isFirstLoad(
  queries: ReadonlyArray<{ isLoading: boolean; isFetched: boolean }>,
): boolean {
  return queries.some((query) => query.isLoading && !query.isFetched)
}

export function estimateFailure(
  statementsError: Error | null,
  estimateError: Error | null,
): string | undefined {
  if (estimateError) return estimateError.message
  if (statementsError) return PERIOD_UNKNOWN
  return undefined
}
