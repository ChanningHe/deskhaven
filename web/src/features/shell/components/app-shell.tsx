import { Outlet } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { useSidebarCollapsed } from '../use-sidebar-collapsed'
import { AppSidebar } from './app-sidebar'
import { TopBar } from './top-bar'

const MAIN_ID = 'main-content'

export function AppShell() {
  const { t } = useTranslation()
  const { collapsed, toggle } = useSidebarCollapsed()

  return (
    <div className="flex h-svh overflow-hidden">
      <a
        href={`#${MAIN_ID}`}
        className="sr-only z-50 rounded-md bg-background px-3 py-2 text-sm focus:not-sr-only focus:absolute focus:top-2 focus:left-2"
      >
        {t('app.skipToContent')}
      </a>
      <AppSidebar collapsed={collapsed} onToggle={toggle} />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <main id={MAIN_ID} tabIndex={-1} className="flex-1 overflow-y-auto outline-none">
          <div className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  )
}
