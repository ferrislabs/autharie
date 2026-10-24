import type { IamEvent } from './iam-flow'

export type IamView<T> =
  | { phase: 'idle' }
  | { phase: 'loading'; slow: boolean }
  | { phase: 'ready'; result: T }
  | { phase: 'failed'; message: string }

export function viewOf<T>(event: IamEvent<T> | null, enabled: boolean): IamView<T> {
  if (!enabled) return { phase: 'idle' }
  if (event === null) return { phase: 'loading', slow: false }

  switch (event.phase) {
    case 'ready':
      return { phase: 'ready', result: event.result }
    case 'failed':
      return { phase: 'failed', message: event.message }
    case 'waiting':
      return { phase: 'loading', slow: event.slow }
    default:
      return { phase: 'loading', slow: false }
  }
}
