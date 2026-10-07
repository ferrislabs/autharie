import { describe, expect, it } from 'vitest'
import { canRemove } from './removal'

describe('which data planes can be removed', () => {
  it('offers removal for a disabled or failed plane', () => {
    expect(canRemove('disabled')).toBe(true)
    expect(canRemove('failed')).toBe(true)
  })

  it('does not offer it for a plane that is in service or on its way', () => {
    expect(canRemove('active')).toBe(false)
    expect(canRemove('draining')).toBe(false)
    expect(canRemove('provisioning')).toBe(false)
  })
})
