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
  /** `nav.sections.*`; the self-service fallback is never titled (it is always alone). */
  title?: 'manage' | 'configure'
  items: readonly NavItem[]
}

type NavSections = readonly NavSection[]

/** Sidebar sections (08 §3.1): daily operations, then configuration. */
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

/** The self-service page; reachable from the account menu (and the palette). */
export const ME_NAV_ITEM: NavItem = { key: 'me', to: '/me', icon: UserRound }

/**
 * The sections `role` may see. A plain user sees none of the admin pages, so
 * the sidebar shows their own page instead of an empty list.
 */
export function navSectionsFor(role: RoleView): NavSections {
  const sections = NAV_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter((item) => hasPermission(role, PAGE_PERMISSIONS[item.key])),
  })).filter((section) => section.items.length > 0)
  return sections.length > 0 ? sections : [{ items: [ME_NAV_ITEM] }]
}
