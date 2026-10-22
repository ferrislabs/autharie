export type StepStatus = 'done' | 'current' | 'todo'

export function stepStatus(index: number, current: number): StepStatus {
  if (index < current) return 'done'
  return index === current ? 'current' : 'todo'
}

export function previousStep(current: number): number {
  return Math.max(current - 1, 0)
}

export function nextStep(current: number, last: number): number {
  return Math.min(current + 1, last)
}
