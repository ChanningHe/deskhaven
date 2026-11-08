import { PanelLeftClose, PanelLeftOpen } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import { Brand } from './brand'
import { SidebarNav } from './sidebar-nav'
import { VersionInfo } from './version-info'

const SIDEBAR_ID = 'app-sidebar'

interface AppSidebarProps {
  collapsed: boolean
  onToggle: () => void
}

/** Desktop sidebar (md and up). Small screens use <MobileNav> instead. */
export function AppSidebar({ collapsed, onToggle }: AppSidebarProps) {
  const { t } = useTranslation()
  const ToggleIcon = collapsed ? PanelLeftOpen : PanelLeftClose

  return (
    <div
      id={SIDEBAR_ID}
      data-collapsed={collapsed}
      className={cn(
        'hidden shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground transition-[width] duration-200 md:flex',
        collapsed ? 'w-14' : 'w-60',
      )}
    >
      <div className={cn('flex h-14 items-center border-b px-3', collapsed && 'justify-center')}>
        <Brand compact={collapsed} />
      </div>
      <SidebarNav collapsed={collapsed} />
      <div
        className={cn(
          'flex items-center gap-2 border-t p-2',
          collapsed ? 'justify-center' : 'justify-between pl-3',
        )}
      >
        {collapsed ? null : <VersionInfo className="min-w-0" />}
        <Button
          variant="ghost"
          size="icon"
          // Fixed name; the state is conveyed by aria-expanded alone.
          aria-label={t('shell.toggleSidebar')}
          aria-expanded={!collapsed}
          aria-controls={SIDEBAR_ID}
          onClick={onToggle}
        >
          <ToggleIcon aria-hidden />
        </Button>
      </div>
    </div>
  )
}
