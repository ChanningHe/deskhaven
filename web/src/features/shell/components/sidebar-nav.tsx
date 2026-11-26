import { Link } from '@tanstack/react-router'
import { Fragment, useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useMe } from '@/features/auth/session'
import { cn } from '@/lib/utils'

import { navSectionsFor } from '../nav-items'
import type { NavItem, NavSection } from '../nav-items'

interface SidebarNavProps {
  collapsed: boolean
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
  const sections = navSectionsFor(role)
  // Titles tell sections apart; a lone section, or the icon rail, needs none.
  const titled = !collapsed && sections.length > 1
  return (
    <nav aria-label={t('nav.label')} className="flex-1 overflow-y-auto p-2">
      {sections.map((section, index) => (
        <Fragment key={section.title ?? index}>
          {index > 0 ? <Separator className="my-2" /> : null}
          <NavSectionList
            section={section}
            title={titled && section.title ? t(`nav.sections.${section.title}`) : undefined}
            collapsed={collapsed}
            {...(onNavigate ? { onNavigate } : {})}
          />
        </Fragment>
      ))}
    </nav>
  )
}

function NavSectionList({
  section,
  title,
  ...link
}: SidebarNavProps & { section: NavSection; title: string | undefined }) {
  const titleId = useId()
  return (
    <>
      {title === undefined ? null : (
        <p
          id={titleId}
          className="px-2.5 pt-1 pb-1.5 text-xs font-medium text-muted-foreground [:root:lang(en)_&]:tracking-wider [:root:lang(en)_&]:uppercase"
        >
          {title}
        </p>
      )}
      <ul
        aria-labelledby={title === undefined ? undefined : titleId}
        className="flex flex-col gap-0.5"
      >
        {section.items.map((item) => (
          <li key={item.key}>
            <SidebarLink item={item} {...link} />
          </li>
        ))}
      </ul>
    </>
  )
}
