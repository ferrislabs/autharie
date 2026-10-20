import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PlanPicker } from './plan-picker'
import { PriceSummary } from './price-summary'
import { VolumePicker } from './volume-picker'

const noop = () => {}

describe('the plan cards', () => {
  const selection = { hosting: 'managed', engine: 'ferriskey', plan: 'business', volume: 10000 } as const

  it('show the three plans with their price', () => {
    const html = renderToStaticMarkup(<PlanPicker selection={selection} onSelect={noop} />)
    expect(html).toContain('Starter')
    expect(html).toContain('Business')
    expect(html).toContain('Scale')
    expect(html).toContain('€99 per month, excl. VAT')
    expect(html).toContain('€225 per month, excl. VAT')
    expect(html).toContain('€349 per month, excl. VAT')
  })

  it('disable Starter above 25,000 accounts and say so', () => {
    const html = renderToStaticMarkup(
      <PlanPicker selection={{ ...selection, volume: 50000 }} onSelect={noop} />,
    )
    expect(html).toContain('Up to 25,000 accounts. Beyond that, choose Business.')
    expect(html).toContain('disabled')
  })

  it('disable a plan that no open offer serves and say so', () => {
    const html = renderToStaticMarkup(
      <PlanPicker selection={selection} onSelect={noop} closedPlans={['scale']} />,
    )
    expect(html).toContain('Not available on your plan.')
    expect(html).toContain('disabled')
  })

  it('use the plans of the hosting', () => {
    const html = renderToStaticMarkup(
      <PlanPicker selection={{ ...selection, hosting: 'byoc' }} onSelect={noop} />,
    )
    expect(html).toContain('BYOC Starter')
    expect(html).toContain('€269 per month, excl. VAT')
  })

  it('say when a plan moved up', () => {
    const html = renderToStaticMarkup(
      <PlanPicker selection={selection} onSelect={noop} movedNotice='moved' />,
    )
    expect(html).toContain('moved')
  })
})

describe('the volume', () => {
  it('is a slider when managed', () => {
    const html = renderToStaticMarkup(
      <VolumePicker hosting='managed' engine='ferriskey' volume={10000} onChange={noop} />,
    )
    expect(html).toContain('type="range"')
    expect(html).toContain('10,000 accounts')
  })

  it('is unlimited in your cloud', () => {
    const html = renderToStaticMarkup(
      <VolumePicker hosting='byoc' engine='ferriskey' volume={10000} onChange={noop} />,
    )
    expect(html).toContain('Unlimited accounts')
    expect(html).not.toContain('type="range"')
  })
})

describe('the summary panel', () => {
  it('shows the price, the billing line and the indicative note', () => {
    const html = renderToStaticMarkup(
      <PriceSummary hosting='managed' price={{ kind: 'price', amount: 99 }} />,
    )
    expect(html).toContain('€99')
    expect(html).toContain('€99 per month, excl. VAT')
    expect(html).toContain('This deployment is billed on its own.')
    expect(html).toContain('Indicative price. Billing for deployments is being set up.')
  })

  it('says Free', () => {
    const html = renderToStaticMarkup(
      <PriceSummary hosting='managed' price={{ kind: 'price', amount: 0 }} />,
    )
    expect(html).toContain('Free')
    expect(html).not.toContain('excl. VAT')
  })

  it('adds the cluster on its own line in your cloud', () => {
    const html = renderToStaticMarkup(
      <PriceSummary
        hosting='byoc'
        price={{ kind: 'price', amount: 269 }}
        clusterEstimate='€30.00 per month, billed by your cloud provider; egress not included'
      />,
    )
    expect(html).toContain('€269 per month, excl. VAT')
    expect(html).toContain('Your cluster')
    expect(html).toContain('billed by your cloud provider')
  })
})
