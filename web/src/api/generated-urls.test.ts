import { describe, expect, it } from 'vitest'

import {
  getRemoveAddressBookShareUrl,
  getUpdateAddressBookPeerUrl,
  getUpdateAddressBookTagUrl,
} from './generated/address-books/address-books'
import { getGetUserUrl } from './generated/users/users'

const GUID = '0190aaaa-0000-7000-8000-000000000001'

describe('generated request URLs', () => {
  it('keep a self-chosen entry id or tag name inside its own path segment', () => {
    const hostile = '../../0190bbbb-0000-7000-8000-000000000002/peers?x=1#y'
    const encoded = encodeURIComponent(hostile)

    expect(getUpdateAddressBookPeerUrl(GUID, hostile)).toBe(
      `/admin/api/v1/address-books/${GUID}/peers/${encoded}`,
    )
    expect(getUpdateAddressBookTagUrl(GUID, hostile)).toBe(
      `/admin/api/v1/address-books/${GUID}/tags/${encoded}`,
    )
    expect(new URL(getUpdateAddressBookTagUrl(GUID, hostile), 'http://x').pathname).toBe(
      `/admin/api/v1/address-books/${GUID}/tags/${encoded}`,
    )
  })

  it('encode the other path parameters, whatever the resource', () => {
    expect(getRemoveAddressBookShareUrl('a/b', 'user', 5)).toBe(
      '/admin/api/v1/address-books/a%2Fb/shares/user/5',
    )
    expect(getGetUserUrl(7)).toBe('/admin/api/v1/users/7')
  })
})
