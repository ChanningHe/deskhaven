import { describe, expect, it } from 'vitest'

import { ADMIN_ME, OWNER_ME } from '@/test/fixtures'

import { tokenView } from './test-data'
import { canRevoke, DEFAULT_EXPIRY, expiryFromChoice, MS_PER_DAY } from './token-rules'

const NOW = 1_800_000_000_000

describe('expiryFromChoice', () => {
  it('preselects 90 days', () => {
    expect(DEFAULT_EXPIRY).toBe('90')
  })

  it('counts days from now and maps "never" to null', () => {
    expect(expiryFromChoice('30', NOW)).toBe(NOW + 30 * MS_PER_DAY)
    expect(expiryFromChoice('90', NOW)).toBe(NOW + 90 * MS_PER_DAY)
    expect(expiryFromChoice('365', NOW)).toBe(NOW + 365 * MS_PER_DAY)
    expect(expiryFromChoice('never', NOW)).toBeNull()
  })
})

describe('canRevoke', () => {
  const adminsToken = tokenView(1, 'a')
  const ownersToken = tokenView(2, 'o', {
    created_by: { id: 9, username: 'boss', role: 'owner' },
  })
  const systemToken = tokenView(3, 's', { created_by: null })

  it('lets an owner revoke any active token', () => {
    for (const token of [adminsToken, ownersToken, systemToken]) {
      expect(canRevoke(OWNER_ME, token)).toBe(true)
    }
  })

  it('keeps owner and system tokens from admins (06 §4.1)', () => {
    expect(canRevoke(ADMIN_ME, adminsToken)).toBe(true)
    expect(canRevoke(ADMIN_ME, ownersToken)).toBe(false)
    expect(canRevoke(ADMIN_ME, systemToken)).toBe(false)
  })

  it('has nothing to revoke on a token that is already revoked or expired', () => {
    expect(canRevoke(OWNER_ME, tokenView(4, 'r', { status: 'revoked' }))).toBe(false)
    expect(canRevoke(OWNER_ME, tokenView(5, 'e', { status: 'expired' }))).toBe(false)
  })
})
