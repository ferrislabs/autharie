import { describe, expect, it } from 'vitest'
import {
  canGoPrevious,
  initialListState,
  listOperation,
  nextPage,
  previousPage,
  submitSearch,
  type ListState,
} from './list-state'

describe('the list state', () => {
  it('opens on the first page, without a search', () => {
    expect(initialListState).toEqual({ search: '', page: 1, seen: [] })
    expect(canGoPrevious(initialListState)).toBe(false)
    expect(listOperation(initialListState)).toEqual({
      type: 'applications.list',
      page: 1,
      limit: 25,
    })
  })

  it('asks for a search only when there is one, trimmed', () => {
    expect(listOperation(submitSearch(initialListState, '  web  '))).toEqual({
      type: 'applications.list',
      page: 1,
      limit: 25,
      search: 'web',
    })
    expect(listOperation(submitSearch(initialListState, '   '))).not.toHaveProperty('search')
  })

  it('goes to the page the answer named and remembers where it came from', () => {
    const second = nextPage(initialListState, 2)
    const third = nextPage(second, 3)

    expect(second).toEqual({ search: '', page: 2, seen: [1] })
    expect(third).toEqual({ search: '', page: 3, seen: [1, 2] })
    expect(canGoPrevious(third)).toBe(true)
  })

  it('does not move when the answer names no next page', () => {
    expect(nextPage(initialListState, undefined)).toBe(initialListState)
  })

  it('walks back through the pages it has seen, in order', () => {
    const third = nextPage(nextPage(initialListState, 2), 3)
    const second = previousPage(third)
    const first = previousPage(second)

    expect(second).toEqual({ search: '', page: 2, seen: [1] })
    expect(first).toEqual({ search: '', page: 1, seen: [] })
    expect(previousPage(first)).toBe(first)
  })

  it('follows the pages the answers named, even when they do not count up by one', () => {
    const state = nextPage(nextPage(initialListState, 4), 9)

    expect(previousPage(state).page).toBe(4)
    expect(previousPage(previousPage(state)).page).toBe(1)
  })

  it('starts again from the first page when a search is submitted', () => {
    const deep: ListState = nextPage(nextPage(initialListState, 2), 3)
    const searched = submitSearch(deep, 'mobile')

    expect(searched).toEqual({ search: 'mobile', page: 1, seen: [] })
    expect(canGoPrevious(searched)).toBe(false)
  })

  it('keeps the search while paging', () => {
    const searched = submitSearch(initialListState, 'web')
    const second = nextPage(searched, 2)

    expect(listOperation(second)).toMatchObject({ page: 2, search: 'web' })
    expect(previousPage(second).search).toBe('web')
  })

  it('makes a new state for the same search, so submitting it again asks again', () => {
    const once = submitSearch(initialListState, 'web')

    expect(submitSearch(once, 'web')).not.toBe(once)
    expect(submitSearch(once, 'web')).toEqual(once)
  })
})
