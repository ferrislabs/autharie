const EUR = new Intl.NumberFormat('en', { style: 'currency', currency: 'EUR' })

export function formatEur(minorUnits: number): string {
  return EUR.format(minorUnits / 100)
}
