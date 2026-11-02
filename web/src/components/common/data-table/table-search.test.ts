import { describe, expect, it } from 'vitest'

import {
  applyTableSearch,
  nextSort,
  parseSort,
  parseTableSearch,
  toListParams,
} from './table-search'
import type { TableSearch } from './table-search'

describe('parseTableSearch', () => {
  const sortable = ['name', 'last_seen']

  it('keeps valid values and leaves defaults implicit', () => {
    expect(
      parseTableSearch({ page: 3, page_size: 50, sort: '-last_seen', q: ' ada ' }, { sortable }),
    ).toEqual({ page: 3, page_size: 50, sort: '-last_seen', q: 'ada' })
    expect(parseTableSearch({ page: '1', page_size: '20' }, { sortable })).toEqual({
      page: undefined,
      page_size: undefined,
      sort: undefined,
      q: undefined,
    })
  })

  it('drops malformed values, unsupported page sizes and unsortable columns', () => {
    expect(
      parseTableSearch({ page: 'two', page_size: 500, sort: 'password_hash', q: 42 }, { sortable }),
    ).toEqual({ page: undefined, page_size: undefined, sort: undefined, q: undefined })
    expect(parseTableSearch({ page: 2.5, sort: '--name' }, { sortable }).page).toBeUndefined()
  })
})

describe('sort helpers', () => {
  it('parses and cycles none -> asc -> desc -> none', () => {
    expect(parseSort('-name')).toEqual({ id: 'name', desc: true })
    expect(parseSort('Name; drop')).toBeUndefined()
    expect(nextSort(undefined, 'name')).toEqual({ id: 'name', desc: false })
    expect(nextSort({ id: 'name', desc: false }, 'name')).toEqual({ id: 'name', desc: true })
    expect(nextSort({ id: 'name', desc: true }, 'name')).toBeUndefined()
    expect(nextSort({ id: 'name', desc: true }, 'os')).toEqual({ id: 'os', desc: false })
  })
})

describe('applyTableSearch', () => {
  interface DeviceSearch extends TableSearch {
    online?: boolean | undefined
  }

  it('resets to page 1 when anything but the page changes', () => {
    const previous: DeviceSearch = { page: 4, sort: 'name' }
    expect(applyTableSearch(previous, { online: true })).toEqual({
      page: undefined,
      sort: 'name',
      online: true,
    })
    expect(applyTableSearch(previous, { page: 5 })).toEqual({ page: 5, sort: 'name' })
    // Re-applying the same value is not a change.
    expect(applyTableSearch(previous, { sort: 'name' })).toEqual({ page: 4, sort: 'name' })
  })

  it('removes defaults and emptied filters without touching the input', () => {
    const previous: DeviceSearch = { q: 'ada', page_size: 50 }
    const next = applyTableSearch(previous, { q: '', page_size: 20 })

    expect(next).toEqual({ q: undefined, page_size: undefined, page: undefined })
    expect(previous).toEqual({ q: 'ada', page_size: 50 })
  })
})

describe('toListParams', () => {
  it('fills in the defaults the API expects', () => {
    expect(toListParams({})).toEqual({ page: 1, page_size: 20 })
    expect(toListParams({ page: 2, sort: '-name', q: 'x' })).toEqual({
      page: 2,
      page_size: 20,
      sort: '-name',
      q: 'x',
    })
  })
})
