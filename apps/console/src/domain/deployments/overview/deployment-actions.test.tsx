import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { DeploymentActions } from './deployment-actions'

const noop = () => {}

function render(over: { status?: 'successful' | 'stopped'; busy?: boolean } = {}) {
  return renderToStaticMarkup(
    <DeploymentActions
      status={over.status ?? 'successful'}
      busy={over.busy ?? false}
      onStop={noop}
      onStart={noop}
    />,
  )
}

describe('the Actions button of a deployment', () => {
  it('opens a menu, named Actions', () => {
    const html = render()
    expect(html).toContain('Actions')
    expect(html).toContain('aria-haspopup="menu"')
  })

  it('can be opened while a deployment runs or is stopped', () => {
    expect(render({ status: 'successful' })).not.toContain('disabled=""')
    expect(render({ status: 'stopped' })).not.toContain('disabled=""')
  })

  it('is disabled, and shows it is working, while a request is out', () => {
    const html = render({ busy: true })
    expect(html).toContain('disabled=""')
    expect(html).toContain('animate-spin')
  })

  it('asks nothing before it is used: the confirmation to stop is not on the page', () => {
    expect(render()).not.toContain('Stop this deployment?')
  })
})
