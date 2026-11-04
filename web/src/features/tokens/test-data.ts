// Fixtures of the API token page tests.

import type { ApiTokenView } from '@/api/generated/model'
import type { MockHandler } from '@/test/mock-api'

export const TOKENS_PATH = '/admin/api/v1/api-tokens'

export function tokenView(
  id: number,
  name: string,
  patch: Partial<ApiTokenView> = {},
): ApiTokenView {
  return {
    id,
    name,
    prefix: `dhk_abcd000${String(id)}`,
    scopes: ['deploy'],
    created_by: { id: 1, username: 'admin', role: 'admin' },
    created_at: 1_700_000_000_000,
    expires_at: null,
    last_used_at: null,
    revoked_at: null,
    status: 'active',
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
