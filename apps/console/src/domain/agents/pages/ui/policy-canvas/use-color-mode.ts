import type { ColorMode } from '@xyflow/react'
import { useEffect, useState } from 'react'

function read(): ColorMode {
  if (typeof document === 'undefined') return 'light'
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

export function useColorMode(): ColorMode {
  const [mode, setMode] = useState<ColorMode>(read)

  useEffect(() => {
    const observer = new MutationObserver(() => setMode(read()))
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
    return () => observer.disconnect()
  }, [])

  return mode
}
