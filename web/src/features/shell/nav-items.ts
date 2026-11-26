import type { LinkProps } from '@tanstack/react-router'
import {
  BookUser,
  Boxes,
  Fingerprint,
  KeyRound,
  LayoutDashboard,
  Monitor,
  MonitorCog,
  ScrollText,
  Settings,
  SlidersHorizontal,
  UserRound,
  Users,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { RoleView } from '@/api/generated/model'
import { hasPermission, PAGE_PERMISSIONS } from '@/features/auth/permissions'
import type { PageKey } from '@/features/auth/permissions'

export interface NavItem {
  key: PageKey
  to: NonNullable<LinkProps['to']>
  icon: LucideIcon
}

export interface NavSection {
  title?: 'manage' | 'configure'
  items: readonly NavItem[]
}

type NavSections = readonly NavSection[]

export const NAV_SECTIONS: NavSections = [
  {
    title: 'manage',
    items: [
      { key: 'overview', to: '/', icon: LayoutDashboard },
      { key: 'devices', to: '/devices', icon: Monitor },
      { key: 'users', to: '/users', icon: Users },
      { key: 'groups', to: '/groups', icon: Boxes },
      { key: 'addressBooks', to: '/address-books', icon: BookUser },
      { key: 'strategies', to: '/strategies', icon: SlidersHorizontal },
      { key: 'audit', to: '/audit', icon: ScrollText },
    ],
  },
  {
    title: 'configure',
    items: [
      { key: 'identity', to: '/identity', icon: Fingerprint },
      { key: 'clientSetup', to: '/client-setup', icon: MonitorCog },
      { key: 'tokens', to: '/tokens', icon: KeyRound },
      { key: 'settings', to: '/settings', icon: Settings },
    ],
  },
]

export const ME_NAV_ITEM: NavItem = { key: 'me', to: '/me', icon: UserRound }

/** A plain user sees none of the admin pages, so the sidebar shows their own page instead. */
export function navSectionsFor(role: RoleView): NavSections {
  const sections = NAV_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter((item) => hasPermission(role, PAGE_PERMISSIONS[item.key])),
  })).filter((section) => section.items.length > 0)
  return sections.length > 0 ? sections : [{ items: [ME_NAV_ITEM] }]
}
