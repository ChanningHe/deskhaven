import { useCallback, useState } from 'react'

import { readStorage, writeStorage } from '@/lib/storage'

export const SIDEBAR_COLLAPSED_STORAGE_KEY = 'deskhaven.sidebar.collapsed'

export function useSidebarCollapsed(): { collapsed: boolean; toggle: () => void } {
  const [collapsed, setCollapsed] = useState(
    () => readStorage(SIDEBAR_COLLAPSED_STORAGE_KEY) === 'true',
  )

  const toggle = useCallback(() => {
    const next = !collapsed
    writeStorage(SIDEBAR_COLLAPSED_STORAGE_KEY, String(next))
    setCollapsed(next)
  }, [collapsed])

  return { collapsed, toggle }
}
