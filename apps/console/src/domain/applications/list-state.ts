import type { IamOperation } from '@/domain/identity/iam/types'

export const PAGE_SIZE = 25

export interface ListState {
  search: string
  page: number
  seen: number[]
}

export const initialListState: ListState = { search: '', page: 1, seen: [] }

export function submitSearch(_state: ListState, term: string): ListState {
  return { search: term.trim(), page: 1, seen: [] }
}

export function nextPage(state: ListState, next: number | undefined): ListState {
  if (next === undefined) return state
  return { ...state, page: next, seen: [...state.seen, state.page] }
}

export function previousPage(state: ListState): ListState {
  const page = state.seen[state.seen.length - 1]
  if (page === undefined) return state
  return { ...state, page, seen: state.seen.slice(0, -1) }
}

export function canGoPrevious(state: ListState): boolean {
  return state.seen.length > 0
}

export function listOperation(state: ListState): IamOperation {
  return {
    type: 'applications.list',
    page: state.page,
    limit: PAGE_SIZE,
    ...(state.search === '' ? {} : { search: state.search }),
  }
}
