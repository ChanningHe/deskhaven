import { describe, expect, it } from 'vitest'

import { auditListParams, hasAuditFilters, parseAuditSearch } from './audit-search'

describe('audit search', () => {
  it('keeps the tab, window, filters and drawer of a valid URL', () => {
    const search = parseAuditSearch({
      tab: 'logins',
      from: '1700000000000',
      to: 1_700_086_400_000,
      user: '5',
      method: 'oidc',
      client: 'console',
      result: 'success',
      item: '12',
    })

    expect(search).toMatchObject({
      tab: 'logins',
      from: 1_700_000_000_000,
      to: 1_700_086_400_000,
      user: 5,
      method: 'oidc',
      client: 'console',
      result: 'success',
      item: 12,
    })
  })

  it('drops malformed values instead of failing', () => {
    const search = parseAuditSearch({
      tab: 'nope',
      from: 'yesterday',
      to: -5,
      user: '0',
      conn_type: '99999',
      sort: 'device',
      item: 'x',
    })

    expect(search.tab).toBeUndefined()
    expect(search.from).toBeUndefined()
    expect(search.to).toBeUndefined()
    expect(search.user).toBeUndefined()
    expect(search.conn_type).toBeUndefined()
    expect(search.sort).toBeUndefined()
    expect(search.item).toBeUndefined()
  })

  it('treats the connections tab as the default and drops filters of other tabs', () => {
    const search = parseAuditSearch({
      tab: 'connections',
      conn_type: '1',
      direction: 'send',
      method: 'oidc',
    })

    expect(search.tab).toBeUndefined()
    expect(search.conn_type).toBe(1)
    expect(search.direction).toBeUndefined()
    expect(search.method).toBeUndefined()
  })

  it('accepts only the creation time as sort', () => {
    expect(parseAuditSearch({ sort: 'created_at' }).sort).toBe('created_at')
    expect(parseAuditSearch({ sort: '-created_at' }).sort).toBe('-created_at')
    expect(parseAuditSearch({ sort: '-id' }).sort).toBeUndefined()
  })

  it('builds the request from the window and the tab filters only', () => {
    const search = parseAuditSearch({
      from: 10,
      user: 3,
      conn_type: 4,
      device: ' 123 ',
      page: 2,
      q: 'pc',
    })

    expect(auditListParams(search)).toEqual({
      page: 2,
      page_size: 20,
      q: 'pc',
      from: 10,
      user: 3,
      conn_type: 4,
      device: '123',
    })
    const logins = parseAuditSearch({ tab: 'logins', user: 3, conn_type: 4, device: '1' })
    expect(auditListParams(logins)).toEqual({
      page: 1,
      page_size: 20,
      user: 3,
    })
  })

  it('knows when a filter is set', () => {
    expect(hasAuditFilters(parseAuditSearch({}))).toBe(false)
    expect(hasAuditFilters(parseAuditSearch({ q: 'x' }))).toBe(false)
    expect(hasAuditFilters(parseAuditSearch({ to: 5 }))).toBe(true)
    expect(hasAuditFilters(parseAuditSearch({ tab: 'files', direction: 'send' }))).toBe(true)
  })
})
