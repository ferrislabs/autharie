import { describe, expect, it } from 'vitest'
import { viewOf } from './iam-view'

describe('what a request looks like to a page', () => {
  it('is idle while there is nothing to ask', () => {
    expect(viewOf(null, false)).toEqual({ phase: 'idle' })
    expect(viewOf({ phase: 'ready', result: 1 }, false)).toEqual({ phase: 'idle' })
  })

  it('is loading before the first event and through the key and the request', () => {
    expect(viewOf(null, true)).toEqual({ phase: 'loading', slow: false })
    expect(viewOf({ phase: 'generating' }, true)).toEqual({ phase: 'loading', slow: false })
    expect(viewOf({ phase: 'requesting' }, true)).toEqual({ phase: 'loading', slow: false })
  })

  it('carries the slow flag while waiting', () => {
    expect(viewOf({ phase: 'waiting', slow: false }, true)).toEqual({ phase: 'loading', slow: false })
    expect(viewOf({ phase: 'waiting', slow: true }, true)).toEqual({ phase: 'loading', slow: true })
  })

  it('hands over the result, or the message', () => {
    expect(viewOf({ phase: 'ready', result: { items: [] } }, true)).toEqual({
      phase: 'ready',
      result: { items: [] },
    })
    expect(viewOf({ phase: 'failed', message: 'no' }, true)).toEqual({
      phase: 'failed',
      message: 'no',
    })
  })
})
