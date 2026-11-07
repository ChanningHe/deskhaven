// Fixtures of the users and groups page tests.

import type { QueryClient } from '@tanstack/react-query'

import type { UserDetail, UserView } from '@/api/generated/model'
import type { MockHandler } from '@/test/mock-api'

export const USERS_PATH = '/admin/api/v1/users'

export function userView(id: number, username: string, patch: Partial<UserView> = {}): UserView {
  return {
    id,
    username,
    display_name: username,
    email: null,
    email_verified: false,
    auth_source: 'local',
    role: 'user',
    status: 'active',
    must_change_password: false,
    totp_enabled: false,
    note: '',
    last_login_at: null,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

/** A list endpoint answering `items` for every page (`total` defaults to their count). */
export function pageOf(items: unknown[], total = items.length): MockHandler {
  return ({ url }) => ({
    body: {
      items,
      total,
      page: Number(url.searchParams.get('page') ?? 1),
      page_size: Number(url.searchParams.get('page_size') ?? 20),
    },
  })
}

export function userDetail(target: UserView): UserDetail {
  return {
    user: target,
    groups: [{ id: 3, name: 'ops' }],
    identities: [],
    sessions: [],
    sessions_total: 0,
  }
}

/** Mutations in the cache whose result still holds a temporary password. */
export function cachedSecrets(queryClient: QueryClient): unknown[] {
  return queryClient
    .getMutationCache()
    .getAll()
    .map((mutation) => mutation.state.data)
    .filter((data) => JSON.stringify(data ?? null).includes('temporary_password'))
}
