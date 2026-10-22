import type { Provider } from '../../types/cloud-provider'

export interface ConnectStep {
  provider: Provider | null
}

export function stepWidth(step: ConnectStep): string {
  return step.provider ? 'sm:max-w-4xl' : 'sm:max-w-lg'
}
