// List state that lives in the URL (08 §4: filters, sort and page go into the
// query string so a view can be shared and the back button works). The keys are
// the admin API's list query parameters (08 §6), so the validated search is
// also the request: `page`, `page_size` (<= 200), `sort` (`field` ascending,
// `-field` descending), `q`, plus page-specific filters.

export const PAGE_SIZES = [10, 20, 50, 100, 200] as const
export const DEFAULT_PAGE_SIZE = 20
const MAX_QUERY_LENGTH = 200
const SORT_PATTERN = /^-?[a-z][a-z0-9_]*$/

export interface TableSearch {
  page?: number | undefined
  page_size?: number | undefined
  sort?: string | undefined
  q?: string | undefined
}

/** TableSearch with defaults filled in: what a list request sends. */
export interface ListParams {
  page: number
  page_size: number
  sort?: string
  q?: string
}

/** One page of a list response (`{items, total, page, page_size}`, 08 §6). */
export interface ListPage<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}

export interface SortState {
  id: string
  desc: boolean
}

function toInteger(value: unknown): number | undefined {
  const number = typeof value === 'string' && value.trim() !== '' ? Number(value) : value
  return typeof number === 'number' && Number.isSafeInteger(number) ? number : undefined
}

export function parseSort(sort: string | undefined): SortState | undefined {
  if (sort === undefined || !SORT_PATTERN.test(sort)) return undefined
  return sort.startsWith('-') ? { id: sort.slice(1), desc: true } : { id: sort, desc: false }
}

export function formatSort(sort: SortState | undefined): string | undefined {
  if (sort === undefined) return undefined
  return sort.desc ? `-${sort.id}` : sort.id
}

/** Sort cycle of a header click: none -> ascending -> descending -> none. */
export function nextSort(current: SortState | undefined, id: string): SortState | undefined {
  if (current?.id !== id) return { id, desc: false }
  return current.desc ? undefined : { id, desc: true }
}

/**
 * Validates the table keys of a raw URL search: anything malformed or out of
 * range is dropped (not an error page), and defaults are left implicit. Use it
 * in a route's `validateSearch`, spreading page-specific filters next to it.
 */
export function parseTableSearch(
  raw: Record<string, unknown>,
  { sortable }: { sortable: readonly string[] },
): TableSearch {
  const page = toInteger(raw.page)
  const pageSize = toInteger(raw.page_size)
  const sort = typeof raw.sort === 'string' ? parseSort(raw.sort) : undefined
  const q = typeof raw.q === 'string' ? raw.q.trim().slice(0, MAX_QUERY_LENGTH) : ''
  return {
    page: page !== undefined && page > 1 ? page : undefined,
    page_size:
      pageSize !== undefined &&
      pageSize !== DEFAULT_PAGE_SIZE &&
      (PAGE_SIZES as readonly number[]).includes(pageSize)
        ? pageSize
        : undefined,
    sort: sort !== undefined && sortable.includes(sort.id) ? formatSort(sort) : undefined,
    q: q === '' ? undefined : q,
  }
}

/** Number of pages for `total` rows (at least 1, so "page 1 of 1" when empty). */
export function pageCount(total: number, pageSize: number): number {
  return Math.max(1, Math.ceil(total / pageSize))
}

export function toListParams({ page, page_size, sort, q }: TableSearch): ListParams {
  return {
    page: page ?? 1,
    page_size: page_size ?? DEFAULT_PAGE_SIZE,
    ...(sort === undefined ? {} : { sort }),
    ...(q === undefined ? {} : { q }),
  }
}

function isDefault(key: string, value: unknown): boolean {
  if (value === undefined || value === '' || value === null) return true
  if (key === 'page') return value === 1
  if (key === 'page_size') return value === DEFAULT_PAGE_SIZE
  return false
}

/**
 * The next search after a change. Any change other than the page itself goes
 * back to page 1 (the old page may no longer exist), and defaults or emptied
 * filters are removed so URLs stay short and canonical. Returns a new object.
 */
export function applyTableSearch<T extends TableSearch>(previous: T, patch: Partial<T>): T {
  const changesMoreThanPage = Object.keys(patch).some(
    (key) => key !== 'page' && patch[key as keyof T] !== previous[key as keyof T],
  )
  const merged: Record<string, unknown> = {
    ...previous,
    ...patch,
    ...(changesMoreThanPage ? { page: undefined } : {}),
  }
  return Object.fromEntries(
    Object.entries(merged).map(([key, value]) => [key, isDefault(key, value) ? undefined : value]),
  ) as unknown as T
}
