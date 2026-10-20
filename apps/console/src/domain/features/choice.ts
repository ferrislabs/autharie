import type { Feature, OverrideChoice } from './types/feature'

export const OVERRIDE_CHOICES: readonly OverrideChoice[] = ['default', 'on', 'off']

export function choiceOf(override: boolean | null | undefined): OverrideChoice {
  if (override === true) return 'on'
  if (override === false) return 'off'
  return 'default'
}

export function overrideOf(choice: OverrideChoice): boolean | null {
  if (choice === 'on') return true
  if (choice === 'off') return false
  return null
}

export function isFeatureEnabled(
  enabled: readonly Feature[] | null | undefined,
  feature: Feature
): boolean {
  return enabled?.includes(feature) === true
}

export function stateLabel(enabled: boolean): string {
  return enabled ? 'On' : 'Off'
}
