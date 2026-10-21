import { ApiRequestError } from '@/api/api.fetch'

export const BILLING_OFF = 'Billing is not enabled on this installation.'

export function accountRefusal(error: Error | null): string | undefined {
  return error?.message
}

export function paymentMethodRefusal(error: Error | null): string | undefined {
  if (error instanceof ApiRequestError && error.status === 503) return BILLING_OFF
  return error?.message
}

export function loadFailure(error: Error | null): string | undefined {
  if (error instanceof ApiRequestError && error.status === 404) return undefined
  return error?.message
}
