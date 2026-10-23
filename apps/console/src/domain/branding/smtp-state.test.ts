import { describe, expect, it } from 'vitest'
import { isNotFerriskey, isPending, smtpBadge } from './smtp-state'

describe('the state badge', () => {
  it('calls pending "Applying" and marks it busy', () => {
    expect(smtpBadge({ phase: 'Pending' })).toEqual({
      label: 'Applying',
      tone: 'progress',
      message: null,
    })
  })

  it('calls applied "Applied"', () => {
    expect(smtpBadge({ phase: 'Applied' })).toEqual({
      label: 'Applied',
      tone: 'success',
      message: null,
    })
  })

  it('keeps the message a provider failed with', () => {
    expect(smtpBadge({ phase: 'Failed', message: 'realm not found' })).toEqual({
      label: 'Failed',
      tone: 'danger',
      message: 'realm not found',
    })
  })
})

describe('whether to keep asking', () => {
  it('asks again only while pending', () => {
    expect(isPending({ phase: 'Pending' })).toBe(true)
    expect(isPending({ phase: 'Applied' })).toBe(false)
    expect(isPending({ phase: 'Failed', message: 'x' })).toBe(false)
    expect(isPending(undefined)).toBe(false)
  })
})

describe('a deployment that is not FerrisKey', () => {
  it('is what a 409 means', () => {
    expect(isNotFerriskey(Object.assign(new Error('x'), { status: 409 }))).toBe(true)
  })

  it('is not any other failure', () => {
    expect(isNotFerriskey(Object.assign(new Error('x'), { status: 403 }))).toBe(false)
    expect(isNotFerriskey(new Error('x'))).toBe(false)
    expect(isNotFerriskey(null)).toBe(false)
  })
})
