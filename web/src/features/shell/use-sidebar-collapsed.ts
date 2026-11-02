import { useCallback, useEffect, useState } from 'react'

import { readStorage, writeStorage } from '@/lib/storage'

export const SIDEBAR_COLLAPSED_STORAGE_KEY = 'deskhaven.sidebar.collapsed'

export function useSidebarCollapsed(): { collapsed: boolean; toggle: () => void } {
  const [collapsed, setCollapsed] = useState(
    () => readStorage(SIDEBAR_COLLAPSED_STORAGE_KEY) === 'true',
  )

  useEffect(() => {
    writeStorage(SIDEBAR_COLLAPSED_STORAGE_KEY, String(collapsed))
  }, [collapsed])

  const toggle = useCallback(() => {
    setCollapsed((previous) => !previous)
  }, [])

  return { collapsed, toggle }
}
