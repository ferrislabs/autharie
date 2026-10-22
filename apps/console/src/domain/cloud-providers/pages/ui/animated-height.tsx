import { useEffect, useRef, useState, type ReactNode } from 'react'

const FOCUS_RING_ROOM = 8

export function AnimatedHeight({ children }: { children: ReactNode }) {
  const inner = useRef<HTMLDivElement>(null)
  const [height, setHeight] = useState<number>()

  useEffect(() => {
    const node = inner.current
    if (!node || typeof ResizeObserver === 'undefined') return

    const observer = new ResizeObserver(() => setHeight(node.offsetHeight + FOCUS_RING_ROOM))
    observer.observe(node)

    return () => observer.disconnect()
  }, [])

  return (
    <div
      style={height === undefined ? undefined : { height }}
      className='-m-1 overflow-hidden p-1 transition-[height] duration-300 ease-out motion-reduce:transition-none'
    >
      <div ref={inner}>{children}</div>
    </div>
  )
}
