// Console roles and page permissions (06 §4.1, 08 §3.2). The server enforces
// every permission; these checks only decide what the console shows.

import type { RoleView } from '@/api/generated/model'
import { HttpError } from '@/lib/http-error'

/** What a page or action needs, named like the admin API's security scopes. */
export type Permission = 'self' | 'admin' | 'owner'

const ROLE_RANK = { user: 0, admin: 1, owner: 2 } as const satisfies Record<RoleView, number>
const PERMISSION_RANK = { self: 0, admin: 1, owner: 2 } as const satisfies Record<
  Permission,
  number
>

export function hasPermission(role: RoleView, permission: Permission): boolean {
  return ROLE_RANK[role] >= PERMISSION_RANK[permission]
}

/** Every console page and the permission it needs (08 §3.2). */
export const PAGE_PERMISSIONS = {
  overview: 'admin',
  devices: 'admin',
  users: 'admin',
  groups: 'admin',
  addressBooks: 'admin',
  strategies: 'admin',
  audit: 'admin',
  identity: 'admin',
  clientSetup: 'admin',
  tokens: 'admin',
  settings: 'owner',
  me: 'self',
} as const satisfies Record<string, Permission>

export type PageKey = keyof typeof PAGE_PERMISSIONS

export const HTTP_FORBIDDEN = 403

/**
 * Throws a 403 HttpError when `role` may not open `page`. Used in route
 * `beforeLoad`; the default route error component renders it as "access denied".
 */
export function requirePagePermission(role: RoleView, page: PageKey): void {
  if (!hasPermission(role, PAGE_PERMISSIONS[page])) {
    throw new HttpError(HTTP_FORBIDDEN, `role ${role} may not open ${page}`)
  }
}
