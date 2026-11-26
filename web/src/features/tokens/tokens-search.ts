import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const TOKEN_SORTABLE = ['name', 'created_at'] as const

export type TokensSearch = TableSearch

export function parseTokensSearch(raw: Record<string, unknown>): TokensSearch {
  return parseTableSearch(raw, { sortable: TOKEN_SORTABLE })
}
