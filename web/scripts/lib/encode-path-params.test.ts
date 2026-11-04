import { describe, expect, it } from 'vitest'

import { encodePathParams } from './encode-path-params.ts'

describe('encodePathParams', () => {
  it('encodes every path parameter of a URL template', () => {
    const source = 'return `/admin/api/v1/address-books/${guid}/peers/${id}`'

    expect(encodePathParams(source)).toBe(
      'return `/admin/api/v1/address-books/${encodeURIComponent(guid)}/peers/${encodeURIComponent(id)}`',
    )
  })

  it('also encodes the templates of query keys', () => {
    const source = 'return [`/admin/api/v1/users/${id}`] as const'

    expect(encodePathParams(source)).toBe(
      'return [`/admin/api/v1/users/${encodeURIComponent(id)}`] as const',
    )
  })

  it('leaves the serialized query string alone', () => {
    const source = '? `/admin/api/v1/users?${stringifiedParams}`'

    expect(encodePathParams(source)).toBe(source)
  })

  it('leaves templates that are not admin API URLs alone', () => {
    const source = 'const label = `${a}/${b}`'

    expect(encodePathParams(source)).toBe(source)
  })

  it('is idempotent', () => {
    const once = encodePathParams('`/admin/api/v1/users/${id}`')

    expect(encodePathParams(once)).toBe(once)
  })
})
