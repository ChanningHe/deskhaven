import { Link } from '@tanstack/react-router'
import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'

import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useMe } from '@/features/auth/session'
import { cn } from '@/lib/utils'

import { navSectionsFor } from '../nav-items'
import type { NavItem } from '../nav-items'

interface SidebarNavProps {
  collapsed: boolean
  /** Called after a link is activated (the mobile drawer closes itself). */
  onNavigate?: () => void
}

const LINK_CLASS =
  'flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm text-sidebar-foreground/80 outline-none transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring data-[status=active]:bg-sidebar-accent data-[status=active]:font-medium data-[status=active]:text-sidebar-accent-foreground'

function SidebarLink({ item, collapsed, onNavigate }: SidebarNavProps & { item: NavItem }) {
  const { t } = useTranslation()
  const label = t(`nav.${item.key}`)
  const Icon = item.icon

  const link = (
    <Link
      to={item.to}
      activeOptions={{ exact: item.to === '/' }}
      className={cn(LINK_CLASS, collapsed && 'justify-center px-0')}
      onClick={onNavigate}
    >
      <Icon aria-hidden className="size-4 shrink-0" />
      <span className={cn(collapsed ? 'sr-only' : 'truncate')}>{label}</span>
    </Link>
  )

  if (!collapsed) return link
  return (
    <Tooltip>
      <TooltipTrigger render={link} />
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  )
}

export function SidebarNav({ collapsed, onNavigate }: SidebarNavProps) {
  const { t } = useTranslation()
  const { role } = useMe()
  return (
    <nav aria-label={t('nav.label')} className="flex-1 overflow-y-auto p-2">
      {navSectionsFor(role).map((section, index) => (
        <Fragment key={section[0]?.key ?? index}>
          {index > 0 ? <Separator className="my-2" /> : null}
          <ul className="flex flex-col gap-0.5">
            {section.map((item) => (
              <li key={item.key}>
                <SidebarLink
                  item={item}
                  collapsed={collapsed}
                  {...(onNavigate ? { onNavigate } : {})}
                />
              </li>
            ))}
          </ul>
        </Fragment>
      ))}
    </nav>
  )
}
