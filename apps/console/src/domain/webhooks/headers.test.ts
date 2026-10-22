import { describe, expect, it } from 'vitest'
import {
  blankHeader,
  checkHeaders,
  headerChanges,
  headersFromNames,
  toHeaderRequests,
} from './headers'

describe('the headers sent on save', () => {
  it('sends a stored header without a value so its value is kept', () => {
    const rows = headersFromNames(['x-token'])
    expect(toHeaderRequests(rows)).toEqual([{ name: 'x-token' }])
  })

  it('sends the new value of a replaced header', () => {
    const [row] = headersFromNames(['x-token'])
    expect(toHeaderRequests([{ ...row, replacing: true, value: 'fresh' }])).toEqual([
      { name: 'x-token', value: 'fresh' },
    ])
  })

  it('sends a new header with its value', () => {
    expect(toHeaderRequests([{ ...blankHeader('k'), name: 'x-new', value: 'v' }])).toEqual([
      { name: 'x-new', value: 'v' },
    ])
  })

  it('leaves a removed header out, which removes it', () => {
    expect(toHeaderRequests([])).toEqual([])
  })

  it('drops blank rows', () => {
    expect(toHeaderRequests([blankHeader('k')])).toEqual([])
  })
})

describe('what a save does to headers', () => {
  it('names unchanged, replaced, added and removed headers', () => {
    const stored = ['a', 'b', 'c']
    const rows = [
      ...headersFromNames(['a']),
      { ...headersFromNames(['b'])[0], replacing: true, value: 'x' },
      { ...blankHeader('n'), name: 'd', value: 'y' },
    ]
    expect(headerChanges(stored, rows)).toEqual({
      unchanged: ['a'],
      replaced: ['b'],
      added: ['d'],
      removed: ['c'],
    })
  })
})

describe('checking headers', () => {
  it('accepts a clean set', () => {
    expect(checkHeaders([{ ...blankHeader('k'), name: 'x-a', value: 'v' }])).toBeNull()
  })

  it('asks for a value on a new header', () => {
    expect(checkHeaders([{ ...blankHeader('k'), name: 'x-a' }])).toMatch(/x-a/)
  })

  it('asks for a value on a replaced header', () => {
    const [row] = headersFromNames(['x-a'])
    expect(checkHeaders([{ ...row, replacing: true }])).toMatch(/x-a/)
  })

  it('asks for a name when only a value was typed', () => {
    expect(checkHeaders([{ ...blankHeader('k'), value: 'v' }])).toMatch(/name/i)
  })

  it('refuses a name HTTP cannot carry', () => {
    expect(checkHeaders([{ ...blankHeader('k'), name: 'bad name', value: 'v' }])).toMatch(
      /bad name/
    )
  })

  it('refuses the same name twice, ignoring case', () => {
    const rows = [
      { ...blankHeader('1'), name: 'X-A', value: 'v' },
      { ...blankHeader('2'), name: 'x-a', value: 'v' },
    ]
    expect(checkHeaders(rows)).toMatch(/twice/)
  })

  it('refuses the signature header, which is ours', () => {
    expect(
      checkHeaders([{ ...blankHeader('k'), name: 'X-FerrisKey-Signature', value: 'v' }])
    ).toMatch(/signature/i)
  })
})
