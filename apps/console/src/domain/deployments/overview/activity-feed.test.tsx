import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ActivityFeed, type ActivityEntry } from './activity-feed'

function render(type: string) {
  const entry: ActivityEntry = { id: 'a1', type, at: Date.now(), outcome: 'done' }

  return renderToStaticMarkup(<ActivityFeed entries={[entry]} />)
}

describe('the activity feed', () => {
  it('names a branding change', () => {
    expect(render('deployment.iam_settings')).toContain('Branding updated')
  })

  it('names an email provider change', () => {
    expect(render('deployment.smtp')).toContain('Email provider updated')
  })

  it('names a login theme change', () => {
    expect(render('deployment.portal_theme')).toContain('Login theme updated')
  })

  it('falls back to the action type for one it does not know', () => {
    expect(render('deployment.unheard_of')).toContain('deployment.unheard_of')
  })
})
