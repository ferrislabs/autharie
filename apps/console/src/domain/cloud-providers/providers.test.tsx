import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { PROVIDERS, providerById } from './providers'

describe('the providers a credential can be created for', () => {
  it('offers Scaleway, drawn with its own logo', () => {
    const scaleway = providerById('scaleway')
    const Logo = scaleway.icon
    const markup = renderToStaticMarkup(<Logo className='h-4 w-4' />)

    expect(PROVIDERS.map((provider) => provider.id)).toEqual(['scaleway'])
    expect(markup).toContain('aria-label="Scaleway"')
    expect(markup).toContain('viewBox="0 0 24 24"')
    expect(markup).toContain('<path d="')
    expect(markup).toContain('h-4 w-4')
  })
})
