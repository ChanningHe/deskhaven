import type { TFunction } from 'i18next'

import { errorCodeOf, errorMessage } from '@/api/error-message'
import { isApiError } from '@/api/api-error'
import type { ErrorCode } from '@/api/generated/model'

import type { ConnectFailure } from '../connect/controller'
import { TIMEOUT_REASON, type CloseCause } from '../handshake/handshake'

/** The codes POST sessions answers with that get their own, remote-specific copy. */
const SESSION_ERRORS = [
  'web_client_disabled',
  'device_offline',
  'device_no_answer',
  'no_relay',
  'too_many_sessions',
  'rate_limited',
  'unavailable',
  'not_found',
] as const satisfies readonly ErrorCode[]

type SessionError = (typeof SESSION_ERRORS)[number]

function isSessionError(code: ErrorCode | undefined): code is SessionError {
  return (SESSION_ERRORS as readonly (ErrorCode | undefined)[]).includes(code)
}

function apiFailureMessage(error: unknown, t: TFunction): string {
  const code = errorCodeOf(error)
  if (code === 'foreign_relay') {
    const relay = isApiError(error) ? error.details?.relay_server : undefined
    return t('remote.errors.foreign_relay', { relay: typeof relay === 'string' ? relay : '?' })
  }
  if (isSessionError(code)) return t(`remote.errors.${code}`)
  return errorMessage(error, t)
}

/** Why connecting failed before the peer session started. */
export function failureMessage(failure: ConnectFailure, t: TFunction): string {
  switch (failure.kind) {
    case 'api':
      return apiFailureMessage(failure.error, t)
    case 'no_decoder':
      return t('remote.errors.noDecoder')
    case 'error':
      return t('remote.errors.failed', { reason: failure.reason })
  }
}

/**
 * Why the peer session ended. Reasons are the session's: ours (closed, timeout, the link)
 * or the peer's own words (close_reason, a login error), shown as given.
 */
export function endingMessage(cause: CloseCause, t: TFunction): string {
  switch (cause.by) {
    case 'user':
      return t('remote.ended.user')
    case 'session_switch':
      return t('remote.ended.sessionSwitch')
    case 'link':
      if (cause.reason === TIMEOUT_REASON) return t('remote.ended.timeout')
      return t('remote.ended.link', { reason: cause.reason })
    case 'peer':
    case 'local':
      return t('remote.ended.other', { reason: cause.reason })
  }
}
