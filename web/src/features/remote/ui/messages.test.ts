import i18n from 'i18next'
import { describe, expect, it } from 'vitest'

import { ApiError } from '@/api/api-error'
import type { ErrorCode } from '@/api/generated/model'

import { endingMessage, failureMessage } from './messages'

const t = i18n.t.bind(i18n)

function apiFailure(status: number, code?: ErrorCode, details?: Record<string, unknown>) {
  return { kind: 'api' as const, error: new ApiError(status, { code, details }) }
}

describe('failureMessage', () => {
  it.each([
    [403, 'web_client_disabled'],
    [409, 'device_offline'],
    [504, 'device_no_answer'],
    [409, 'no_relay'],
    [429, 'too_many_sessions'],
    [429, 'rate_limited'],
    [503, 'unavailable'],
    [404, 'not_found'],
  ] as const)('gives %i %s its remote-specific message', (status, code) => {
    expect(failureMessage(apiFailure(status, code), t)).toBe(t(`remote.errors.${code}`))
  })

  it('names the relay the device uses instead of ours', () => {
    const failure = apiFailure(409, 'foreign_relay', { relay_server: 'relay.other.example' })
    expect(failureMessage(failure, t)).toBe(
      t('remote.errors.foreign_relay', { relay: 'relay.other.example' }),
    )
  })

  it('falls back to the generic message of other errors', () => {
    expect(failureMessage(apiFailure(500, 'internal'), t)).toBe(t('errors.codes.internal'))
    expect(failureMessage({ kind: 'api', error: new TypeError('fetch') }, t)).toBe(
      t('errors.network'),
    )
  })

  it('explains a browser without a decoder and a session that cannot start', () => {
    expect(failureMessage({ kind: 'no_decoder' }, t)).toBe(t('remote.errors.noDecoder'))
    expect(failureMessage({ kind: 'error', reason: 'bad key' }, t)).toBe(
      t('remote.errors.failed', { reason: 'bad key' }),
    )
  })
})

describe('endingMessage', () => {
  it.each([
    [{ by: 'user', reason: '' }, 'remote.ended.user', undefined],
    [{ by: 'link', reason: 'timeout' }, 'remote.ended.timeout', undefined],
    [
      { by: 'link', reason: 'relay connection closed (code 1006)' },
      'remote.ended.link',
      'relay connection closed (code 1006)',
    ],
    [
      { by: 'session_switch', reason: 'connection closed' },
      'remote.ended.sessionSwitch',
      undefined,
    ],
    [
      { by: 'local', reason: 'video decoding failed: x' },
      'remote.ended.other',
      'video decoding failed: x',
    ],
    [{ by: 'restart', reason: 'connection closed' }, 'remote.ended.restarting', undefined],
  ] as const)('explains %o', (cause, key, reason) => {
    expect(endingMessage(cause, t)).toBe(reason === undefined ? t(key) : t(key, { reason }))
  })

  it.each([
    'closed',
    'timeout',
    'connection closed',
    'relay connection failed',
    'the device is restarting',
  ])('shows the peer\'s "%s" as its own words, never as ours', (reason) => {
    expect(endingMessage({ by: 'peer', reason }, t)).toBe(t('remote.ended.other', { reason }))
  })
})
