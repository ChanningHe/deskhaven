// URL state of the API token page: the table keys (the admin API's list query, 08 §6).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const TOKEN_SORTABLE = ['name', 'created_at'] as const

export type TokensSearch = TableSearch

/** Validates the raw URL search; anything malformed is dropped. */
export function parseTokensSearch(raw: Record<string, unknown>): TokensSearch {
  return parseTableSearch(raw, { sortable: TOKEN_SORTABLE })
}
