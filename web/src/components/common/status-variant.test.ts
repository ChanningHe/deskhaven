import { describe, expect, it } from 'vitest'

import {
  accountStatusVariant,
  flagVariant,
  loginResultVariant,
  tokenStatusVariant,
} from './status-variant'

describe('status variants', () => {
  it('colours account statuses: usable, stopped by someone, pending', () => {
    expect(accountStatusVariant('active')).toBe('success')
    expect(accountStatusVariant('disabled')).toBe('secondary')
    expect(accountStatusVariant('unverified')).toBe('outline')
  })

  it('colours token statuses: only an active token is good', () => {
    expect(tokenStatusVariant('active')).toBe('success')
    expect(tokenStatusVariant('revoked')).toBe('secondary')
    expect(tokenStatusVariant('expired')).toBe('secondary')
  })

  it('colours login results: success, locked needs attention, the rest failed', () => {
    expect(loginResultVariant('success')).toBe('success')
    expect(loginResultVariant('locked')).toBe('warning')
    expect(loginResultVariant('invalid_credentials')).toBe('destructive')
    expect(loginResultVariant('disabled')).toBe('destructive')
    expect(loginResultVariant('some_future_reason')).toBe('destructive')
  })

  it('colours an on/off fact (2FA enabled, email verified, current session)', () => {
    expect(flagVariant(true)).toBe('success')
    expect(flagVariant(false)).toBe('outline')
  })
})
