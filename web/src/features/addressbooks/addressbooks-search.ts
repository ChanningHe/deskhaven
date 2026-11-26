import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const BOOK_SORTABLE = ['name', 'created_at', 'updated_at'] as const

export const BOOK_TABS = ['entries', 'tags', 'shares', 'details'] as const
export type BookTab = (typeof BOOK_TABS)[number]

export const DEFAULT_BOOK_TAB = 'entries' as const satisfies BookTab

const GUID_PATTERN = /^[0-9a-zA-Z-]{1,64}$/

export interface AddressBooksSearch extends TableSearch {
  book?: string | undefined
  tab?: Exclude<BookTab, typeof DEFAULT_BOOK_TAB> | undefined
}

export function parseAddressBooksSearch(raw: Record<string, unknown>): AddressBooksSearch {
  const tab = BOOK_TABS.find((candidate) => candidate === raw.tab)
  return {
    ...parseTableSearch(raw, { sortable: BOOK_SORTABLE }),
    book: typeof raw.book === 'string' && GUID_PATTERN.test(raw.book) ? raw.book : undefined,
    tab: tab === undefined || tab === 'entries' ? undefined : tab,
  }
}
