import { Fragment, type ReactNode } from 'react'

export function emphasised(text: string): ReactNode {
  return text.split('**').map((part, index) =>
    index % 2 === 1 ? (
      <strong key={index} className='font-medium text-foreground'>
        {part}
      </strong>
    ) : (
      <Fragment key={index}>{part}</Fragment>
    ),
  )
}
