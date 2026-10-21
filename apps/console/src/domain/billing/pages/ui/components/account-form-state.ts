import type { Schemas } from '@/api/api.client'

export type AccountForm = {
  name: string
  email: string
  line1: string
  line2: string
  postal_code: string
  city: string
  country: string
  vat_number: string
  siren: string
}

export function formFrom(account: Schemas.BillingAccountResponse | null): AccountForm {
  return {
    name: account?.name ?? '',
    email: account?.email ?? '',
    line1: account?.address.line1 ?? '',
    line2: account?.address.line2 ?? '',
    postal_code: account?.address.postal_code ?? '',
    city: account?.address.city ?? '',
    country: account?.address.country ?? '',
    vat_number: account?.vat_number ?? '',
    siren: account?.siren ?? '',
  }
}

function optional(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

export function requestFrom(form: AccountForm): Schemas.BillingAccountRequest {
  return {
    name: form.name.trim(),
    email: form.email.trim(),
    address: {
      line1: form.line1.trim(),
      line2: optional(form.line2),
      postal_code: form.postal_code.trim(),
      city: form.city.trim(),
      country: form.country.trim().toUpperCase(),
    },
    vat_number: optional(form.vat_number),
    siren: optional(form.siren),
  }
}

export function isFilled(form: AccountForm): boolean {
  const request = requestFrom(form)
  return (
    request.name !== '' &&
    request.email !== '' &&
    request.address.line1 !== '' &&
    request.address.postal_code !== '' &&
    request.address.city !== '' &&
    request.address.country.length === 2
  )
}
