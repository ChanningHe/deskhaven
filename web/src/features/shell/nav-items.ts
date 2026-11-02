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
  Users,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { TranslationResources } from '@/i18n/resources'

export type NavKey = Exclude<keyof TranslationResources['nav'], 'label'>

export interface NavItem {
  key: NavKey
  to: NonNullable<LinkProps['to']>
  icon: LucideIcon
}

/** Sidebar sections (08 §3.1): daily operations, then configuration. */
export const NAV_SECTIONS: readonly (readonly NavItem[])[] = [
  [
    { key: 'overview', to: '/', icon: LayoutDashboard },
    { key: 'devices', to: '/devices', icon: Monitor },
    { key: 'users', to: '/users', icon: Users },
    { key: 'groups', to: '/groups', icon: Boxes },
    { key: 'addressBooks', to: '/address-books', icon: BookUser },
    { key: 'strategies', to: '/strategies', icon: SlidersHorizontal },
    { key: 'audit', to: '/audit', icon: ScrollText },
  ],
  [
    { key: 'identity', to: '/identity', icon: Fingerprint },
    { key: 'clientSetup', to: '/client-setup', icon: MonitorCog },
    { key: 'tokens', to: '/tokens', icon: KeyRound },
    { key: 'settings', to: '/settings', icon: Settings },
  ],
]
