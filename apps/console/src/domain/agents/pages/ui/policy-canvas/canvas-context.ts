import { createContext, useContext } from 'react'
import type { Focus } from '../../../canvas-model'

export interface RuleStatus {
  issues: string[]
  changed: boolean
}

export interface CanvasUi {
  readOnly: boolean
  focus: Focus | null
  status: Record<string, RuleStatus>
}

export const CanvasUiContext = createContext<CanvasUi>({
  readOnly: true,
  focus: null,
  status: {},
})

export function useCanvasUi(): CanvasUi {
  return useContext(CanvasUiContext)
}

export function useDimmedBox(id: string): boolean {
  const { focus } = useCanvasUi()
  return focus !== null && !focus.boxes.has(id)
}

export function useDimmedRule(ruleId: string): boolean {
  const { focus } = useCanvasUi()
  return focus !== null && !focus.rules.has(ruleId)
}
