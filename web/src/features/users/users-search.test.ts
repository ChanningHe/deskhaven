import { describe, expect, it } from 'vitest'

import { ADMIN_ME, OWNER_ME, USER_ME } from '@/test/fixtures'

import { assignableRoles, canDelete, canManage } from './user-rules'
import { parseUsersSearch, usersListParams } from './users-search'

describe('users search', () => {
  it('keeps valid filters and the drawer id, drops anything else', () => {
    const search = parseUsersSearch({
      role: 'admin',
      status: 'gone',
      auth_source: 'ldap',
      user: '12',
      sort: '-password_hash',
      page: '3',
    })
    expect(search).toEqual({
      page: 3,
      page_size: undefined,
      sort: undefined,
      q: undefined,
      role: 'admin',
      status: undefined,
      auth_source: 'ldap',
      user: 12,
    })
    expect(parseUsersSearch({ user: '-1' }).user).toBeUndefined()
  })

  it('sends filters but never the drawer id to the list endpoint', () => {
    const params = usersListParams({ role: 'user', user: 4 }, { page: 1, page_size: 20 })
    expect(params).toEqual({ page: 1, page_size: 20, role: 'user' })
  })
})

describe('user rules', () => {
  it('lets admins manage everyone but owners, and owners everyone', () => {
    expect(canManage(ADMIN_ME, 'owner')).toBe(false)
    expect(canManage(ADMIN_ME, 'admin')).toBe(true)
    expect(canManage(OWNER_ME, 'owner')).toBe(true)
    expect(canManage(USER_ME, 'user')).toBe(false)
    expect(assignableRoles(ADMIN_ME)).toEqual(['user', 'admin'])
    expect(assignableRoles(OWNER_ME)).toEqual(['user', 'admin', 'owner'])
  })

  it('never lets anyone delete their own account', () => {
    expect(canDelete(OWNER_ME, { id: OWNER_ME.id, role: 'owner' })).toBe(false)
    expect(canDelete(OWNER_ME, { id: 99, role: 'owner' })).toBe(true)
  })
})
