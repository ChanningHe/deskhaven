import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import type { ReactNode } from 'react'

import { readStorage, writeStorage } from '@/lib/storage'

import { applyTheme, DARK_MEDIA_QUERY, isTheme, resolveTheme, THEME_STORAGE_KEY } from './theme'
import type { Theme } from './theme'
import { ThemeContext } from './theme-context'

function readStoredTheme(): Theme {
  const stored = readStorage(THEME_STORAGE_KEY)
  return isTheme(stored) ? stored : 'system'
}

function darkMediaQuery(): MediaQueryList | null {
  return typeof window.matchMedia === 'function' ? window.matchMedia(DARK_MEDIA_QUERY) : null
}

function subscribePrefersDark(onChange: () => void): () => void {
  const query = darkMediaQuery()
  query?.addEventListener('change', onChange)
  return () => query?.removeEventListener('change', onChange)
}

function getPrefersDark(): boolean {
  return darkMediaQuery()?.matches ?? false
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme)
  const prefersDark = useSyncExternalStore(subscribePrefersDark, getPrefersDark)
  const resolvedTheme = resolveTheme(theme, prefersDark)

  useEffect(() => {
    applyTheme(document.documentElement, resolvedTheme)
  }, [resolvedTheme])

  const setTheme = useCallback((next: Theme) => {
    writeStorage(THEME_STORAGE_KEY, next)
    setThemeState(next)
  }, [])

  const value = useMemo(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  )

  return <ThemeContext value={value}>{children}</ThemeContext>
}
