import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { DeploymentStatusBadge } from './deployment-status'

describe('DeploymentStatusBadge', () => {
  it('names a stopped deployment as stopped', () => {
    const html = renderToStaticMarkup(<DeploymentStatusBadge status='stopped' />)

    expect(html).toContain('Stopped')
  })

  it('shows a stopped deployment as inactive, neither running nor failing', () => {
    const html = renderToStaticMarkup(<DeploymentStatusBadge status='stopped' />)

    expect(html).toContain('bg-muted')
    expect(html).not.toContain('green')
    expect(html).not.toContain('red')
    expect(html).not.toContain('amber')
    expect(html).not.toContain('blue')
  })

  it('keeps naming a running deployment as running', () => {
    expect(renderToStaticMarkup(<DeploymentStatusBadge status='successful' />)).toContain(
      'Running',
    )
  })
})
