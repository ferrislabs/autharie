import { describe, expect, it } from 'vitest'
import type { Schemas } from '@/api/api.client'
import { ACTIVE_BADGE, isApplying, themeStateBadge, themeStateSummary } from './theme-state'

const pending: Schemas.ThemeSync = { phase: 'Pending', revision: 3 }
const applied: Schemas.ThemeSync = { phase: 'Applied', revision: 3 }
const failed: Schemas.ThemeSync = {
  phase: 'Failed',
  message: 'colors.error: not a colour',
  revision: 3,
}

describe('what became of the tokens sent to FerrisKey', () => {
  it('is Applying while pending', () => {
    expect(themeStateBadge(pending)).toEqual({ label: 'Applying', tone: 'progress', message: null })
  })

  it('is Applied once FerrisKey took them', () => {
    expect(themeStateBadge(applied)).toEqual({ label: 'Applied', tone: 'success', message: null })
  })

  it('is Failed and says why', () => {
    expect(themeStateBadge(failed)).toEqual({
      label: 'Failed',
      tone: 'danger',
      message: 'colors.error: not a colour',
    })
  })

  it('is polled while it is pending, and only then', () => {
    expect(isApplying(pending)).toBe(true)
    expect(isApplying(applied)).toBe(false)
    expect(isApplying(failed)).toBe(false)
    expect(isApplying(null)).toBe(false)
    expect(isApplying(undefined)).toBe(false)
  })

  it('says which look it is about', () => {
    expect(themeStateSummary('Autumn')).toBe('Active theme: Autumn')
    expect(themeStateSummary(null)).toBe('FerrisKey’s default look')
  })
})

describe('the active badge', () => {
  it('stands out', () => {
    expect(ACTIVE_BADGE).toEqual({ label: 'Active', tone: 'accent' })
  })
})
