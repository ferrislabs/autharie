import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { emphasised } from './emphasis'

describe('the emphasis in a guide', () => {
  it('bolds what is between double asterisks, and nothing else', () => {
    const html = renderToStaticMarkup(<p>{emphasised('Click **Create** now, then **Save**.')}</p>)
    expect(html).toBe(
      '<p>Click <strong class="font-medium text-foreground">Create</strong> now, then <strong class="font-medium text-foreground">Save</strong>.</p>',
    )
  })

  it('leaves a text without markers as it is', () => {
    expect(renderToStaticMarkup(<p>{emphasised('Plain text.')}</p>)).toBe('<p>Plain text.</p>')
  })
})
