import type { Hosting, Price } from './model'

const EUR = new Intl.NumberFormat('en-GB', {
  style: 'currency',
  currency: 'EUR',
  maximumFractionDigits: 0,
})

export const BILLED_ON_ITS_OWN = 'This deployment is billed on its own.'
export const INDICATIVE_NOTE = 'Indicative price. Billing for deployments is being set up.'

export function formatAmount(price: Price): string {
  if (price.kind === 'unavailable') return '—'
  return price.amount === 0 ? 'Free' : EUR.format(price.amount)
}

export function describePrice(price: Price): string {
  if (price.kind === 'price' && price.amount === 0) return 'Free'
  if (price.kind === 'unavailable') return '—'
  return `${EUR.format(price.amount)} per month, excl. VAT`
}

export interface SummaryLine {
  label: string
  value: string
}

export function summaryLines(hosting: Hosting, price: Price, clusterEstimate?: string): SummaryLine[] {
  const lines: SummaryLine[] = [{ label: 'Autharie', value: describePrice(price) }]

  if (hosting === 'byoc') {
    lines.push({
      label: 'Your cluster',
      value: clusterEstimate ?? 'Estimated above, billed by your cloud provider',
    })
  }

  return lines
}

export function describeVolume(hosting: Hosting, volume: number): string {
  return hosting === 'byoc' ? 'Unlimited accounts' : `${new Intl.NumberFormat('en-GB').format(volume)} accounts`
}
