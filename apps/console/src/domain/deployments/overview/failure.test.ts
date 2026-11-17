import { describe, expect, it } from 'vitest'
import { FAILED_WITHOUT_REASON, failureNotice } from './failure'

describe('the reason shown for a failed deployment', () => {
  it('shows what the data plane said', () => {
    expect(failureNotice('failed', 'ImagePullBackOff')).toBe('ImagePullBackOff')
  })

  it('says so when the data plane gave none', () => {
    expect(failureNotice('failed', null)).toBe(FAILED_WITHOUT_REASON)
    expect(failureNotice('failed', undefined)).toBe(FAILED_WITHOUT_REASON)
    expect(failureNotice('failed', '  ')).toBe(FAILED_WITHOUT_REASON)
  })

  it('shows nothing for a deployment that has not failed', () => {
    expect(failureNotice('successful', 'stale')).toBeUndefined()
    expect(failureNotice('in_progress', null)).toBeUndefined()
  })
})
