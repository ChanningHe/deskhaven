// URL state of the address books page: the table keys, the open drawer (`book`, a guid) and
// the drawer's tab (`tab`; absent: the entries).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const BOOK_SORTABLE = ['name', 'created_at', 'updated_at'] as const

export const BOOK_TABS = ['entries', 'tags', 'shares', 'details'] as const
export type BookTab = (typeof BOOK_TABS)[number]

/** The tab shown when the URL names none. */
export const DEFAULT_BOOK_TAB = 'entries' as const satisfies BookTab

/** Address book guids are UUIDs; anything else cannot name a book. */
const GUID_PATTERN = /^[0-9a-zA-Z-]{1,64}$/

export interface AddressBooksSearch extends TableSearch {
  /** Guid of the book shown in the drawer. */
  book?: string | undefined
  /** Drawer tab other than the default. */
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
