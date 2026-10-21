import { describe, expect, it } from 'vitest'
import { formFrom, isFilled, requestFrom } from './account-form-state'

const filled = {
  name: ' Acme ',
  email: 'billing@acme.test',
  line1: '1 rue de la Paix',
  line2: '  ',
  postal_code: '75002',
  city: 'Paris',
  country: 'fr',
  vat_number: '',
  siren: '123456789',
}

describe('the account form', () => {
  it('starts empty without an account', () => {
    expect(isFilled(formFrom(null))).toBe(false)
    expect(formFrom(null).country).toBe('')
  })

  it('trims the request, upper-cases the country and sends null for what is left blank', () => {
    expect(requestFrom(filled)).toEqual({
      name: 'Acme',
      email: 'billing@acme.test',
      address: {
        line1: '1 rue de la Paix',
        line2: null,
        postal_code: '75002',
        city: 'Paris',
        country: 'FR',
      },
      vat_number: null,
      siren: '123456789',
    })
  })

  it('needs a name, an e-mail, an address and a two-letter country', () => {
    expect(isFilled(filled)).toBe(true)
    expect(isFilled({ ...filled, name: ' ' })).toBe(false)
    expect(isFilled({ ...filled, email: '' })).toBe(false)
    expect(isFilled({ ...filled, line1: '' })).toBe(false)
    expect(isFilled({ ...filled, postal_code: '' })).toBe(false)
    expect(isFilled({ ...filled, city: '' })).toBe(false)
    expect(isFilled({ ...filled, country: 'FRA' })).toBe(false)
  })

  it('reads an account back into the form', () => {
    const form = formFrom({
      name: 'Acme',
      email: 'billing@acme.test',
      address: { line1: 'a', line2: 'b', postal_code: 'c', city: 'd', country: 'FR' },
      vat_number: 'FR1',
      siren: null,
      payment_method: 'valid',
      complete: true,
    })
    expect(form).toMatchObject({ line2: 'b', vat_number: 'FR1', siren: '' })
  })
})
