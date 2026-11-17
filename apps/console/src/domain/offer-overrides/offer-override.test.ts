import { describe, expect, it } from 'vitest'
import { offerChoiceOf, offerOverrideOf, openLabel } from './offer-override'

describe('the override of an offer', () => {
  it('reads null as the default', () => {
    expect(offerChoiceOf(null)).toBe('default')
    expect(offerChoiceOf(undefined)).toBe('default')
  })

  it('reads true as open and false as closed', () => {
    expect(offerChoiceOf(true)).toBe('open')
    expect(offerChoiceOf(false)).toBe('closed')
  })

  it('goes back from a choice to what is sent', () => {
    expect(offerOverrideOf('default')).toBeNull()
    expect(offerOverrideOf('open')).toBe(true)
    expect(offerOverrideOf('closed')).toBe(false)
  })

  it('labels a state', () => {
    expect(openLabel(true)).toBe('Open')
    expect(openLabel(false)).toBe('Closed')
  })
})
