import { BaseEdge, getBezierPath, type EdgeProps } from '@xyflow/react'
import { useDimmedRule } from './canvas-context'
import type { CanvasEdge } from './flow-elements'

export function LinkEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  markerEnd,
  style,
  data,
  selected,
}: EdgeProps<CanvasEdge>) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition })
  const dimmed = useDimmedRule(data?.ruleId ?? '')
  const resting = data?.enabled === false ? 0.7 : 1

  return (
    <BaseEdge
      id={id}
      path={path}
      markerEnd={markerEnd}
      style={{
        ...style,
        strokeWidth: selected ? 4 : style?.strokeWidth,
        opacity: dimmed ? 0.15 : resting,
        cursor: 'pointer',
      }}
      interactionWidth={24}
    />
  )
}
