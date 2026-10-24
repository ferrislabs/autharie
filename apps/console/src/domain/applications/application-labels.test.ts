import { describe, expect, it } from 'vitest'
import { formatSeconds, formatWhen, humanise } from './application-labels'

describe('application labels', () => {
  it('turns what FerrisKey calls things into words', () => {
    expect(humanise('confidential')).toBe('Confidential')
    expect(humanise('openid-connect')).toBe('Openid connect')
    expect(humanise('metadata_document')).toBe('Metadata document')
  })

  it('keeps a value it cannot make words of', () => {
    expect(humanise('')).toBe('')
    expect(humanise('---')).toBe('---')
  })

  it('writes a date and keeps what is not one', () => {
    expect(formatWhen('2026-10-23T10:00:00Z')).toMatch(/^2026-10-2[234] \d\d:\d\d$/)
    expect(formatWhen('yesterday')).toBe('yesterday')
  })

  it('writes a lifetime in seconds', () => {
    expect(formatSeconds(300)).toBe('300 s')
  })
})
