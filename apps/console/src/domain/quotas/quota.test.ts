import { describe, expect, it } from 'vitest'
import {
  UNLIMITED,
  describeQuota,
  describeUsage,
  hasRoom,
  limited,
  parseLimit,
  sourceLabel,
} from './quota'

describe('parsing what was typed in the override field', () => {
  it('accepts a whole number', () => {
    expect(parseLimit('25')).toEqual({ ok: true, quota: limited(25) })
  })

  it('accepts zero, which allows nothing', () => {
    expect(parseLimit('0')).toEqual({ ok: true, quota: limited(0) })
  })

  it('ignores the spaces around it', () => {
    expect(parseLimit('  7 ')).toEqual({ ok: true, quota: limited(7) })
  })

  it.each(['', ' ', '-1', '1.5', '1e3', 'ten', '+3'])(
    'refuses %j rather than rounding it',
    (text) => {
      expect(parseLimit(text).ok).toBe(false)
    }
  )

  it('refuses a number the column cannot hold', () => {
    expect(parseLimit('2147483648').ok).toBe(false)
    expect(parseLimit('2147483647').ok).toBe(true)
  })
})

describe('describing a quota', () => {
  it('writes a limit as its number and no limit as Unlimited', () => {
    expect(describeQuota(limited(5))).toBe('5')
    expect(describeQuota(limited(0))).toBe('0')
    expect(describeQuota(UNLIMITED)).toBe('Unlimited')
  })

  it('says whether a row is the default or an override', () => {
    expect(sourceLabel(null)).toBe('Default')
    expect(sourceLabel(undefined)).toBe('Default')
    expect(sourceLabel(UNLIMITED)).toBe('Override')
    expect(sourceLabel(limited(0))).toBe('Override')
  })

  it('writes usage as n of max, or the count alone when nothing limits it', () => {
    expect(describeUsage(2, limited(5))).toBe('2 of 5')
    expect(describeUsage(2, UNLIMITED)).toBe('2')
  })
})

describe('whether another can be created', () => {
  it('has room below the limit and none at it', () => {
    expect(hasRoom(4, limited(5))).toBe(true)
    expect(hasRoom(5, limited(5))).toBe(false)
    expect(hasRoom(9, limited(5))).toBe(false)
  })

  it('has no room at all under a limit of zero', () => {
    expect(hasRoom(0, limited(0))).toBe(false)
  })

  it('always has room when unlimited, or when the quota is not known yet', () => {
    expect(hasRoom(10_000, UNLIMITED)).toBe(true)
    expect(hasRoom(10_000, undefined)).toBe(true)
  })
})
