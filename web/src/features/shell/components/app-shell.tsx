import { Outlet } from '@tanstack/react-router'
import { useRef } from 'react'
import type { MouseEvent, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { useSidebarCollapsed } from '../use-sidebar-collapsed'
import { AppSidebar } from './app-sidebar'
import { RouteAnnouncer } from './route-announcer'
import { TopBar } from './top-bar'

const MAIN_ID = 'main-content'

interface AppShellProps {
  /** Content in place of the matched child route (used by the not-found page). */
  children?: ReactNode
}

export function AppShell({ children }: AppShellProps) {
  const { t } = useTranslation()
  const { collapsed, toggle } = useSidebarCollapsed()
  const mainRef = useRef<HTMLElement>(null)

  // Focus <main> directly: following the #hash would add a history entry and
  // fight the router's own URL handling.
  const skipToContent = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault()
    mainRef.current?.focus()
  }

  return (
    <div className="flex h-svh overflow-hidden">
      <a
        href={`#${MAIN_ID}`}
        onClick={skipToContent}
        className="sr-only z-50 rounded-md bg-background px-3 py-2 text-sm focus:not-sr-only focus:absolute focus:top-2 focus:left-2"
      >
        {t('app.skipToContent')}
      </a>
      <AppSidebar collapsed={collapsed} onToggle={toggle} />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <main
          ref={mainRef}
          id={MAIN_ID}
          tabIndex={-1}
          className="flex-1 overflow-y-auto outline-none"
        >
          <div className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
            {children ?? <Outlet />}
          </div>
        </main>
      </div>
      <RouteAnnouncer mainRef={mainRef} />
    </div>
  )
}
