import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'
import type { AuthSourceView, ListUsersParams, RoleView, StatusView } from '@/api/generated/model'

export const USER_SORTABLE = [
  'username',
  'display_name',
  'role',
  'status',
  'created_at',
  'last_login_at',
] as const

export const ROLES: readonly RoleView[] = ['owner', 'admin', 'user']
export const STATUSES: readonly StatusView[] = ['active', 'disabled', 'unverified']
export const AUTH_SOURCES: readonly AuthSourceView[] = ['local', 'ldap', 'oidc']

export interface UsersSearch extends TableSearch {
  role?: RoleView | undefined
  status?: StatusView | undefined
  auth_source?: AuthSourceView | undefined
  user?: number | undefined
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[]): T | undefined {
  return allowed.find((candidate) => candidate === value)
}

function positiveId(value: unknown): number | undefined {
  const number = typeof value === 'string' ? Number(value) : value
  return typeof number === 'number' && Number.isSafeInteger(number) && number > 0
    ? number
    : undefined
}

export function parseUsersSearch(raw: Record<string, unknown>): UsersSearch {
  return {
    ...parseTableSearch(raw, { sortable: USER_SORTABLE }),
    role: oneOf(raw.role, ROLES),
    status: oneOf(raw.status, STATUSES),
    auth_source: oneOf(raw.auth_source, AUTH_SOURCES),
    user: positiveId(raw.user),
  }
}

export function hasUserFilters(search: UsersSearch): boolean {
  return (
    search.role !== undefined || search.status !== undefined || search.auth_source !== undefined
  )
}

export function usersListParams(search: UsersSearch, base: ListUsersParams): ListUsersParams {
  return {
    ...base,
    ...(search.role === undefined ? {} : { role: search.role }),
    ...(search.status === undefined ? {} : { status: search.status }),
    ...(search.auth_source === undefined ? {} : { auth_source: search.auth_source }),
  }
}
