// Error code -> translated message (08 §6: the console translates by `code`,
// never shows the server's English `message`). Exhaustive over the generated
// ErrorCode union: a new backend code fails `tsc` here until it gets a key.

import type { TFunction } from 'i18next'

import { httpStatusOf } from '@/lib/http-error'

import { isApiError } from './api-error'
import type { ErrorCode } from './generated/model'

type CodeKey = `errors.codes.${ErrorCode}`

const CODE_KEYS: { readonly [Code in ErrorCode]: `errors.codes.${Code}` } = {
  validation_failed: 'errors.codes.validation_failed',
  unauthorized: 'errors.codes.unauthorized',
  forbidden: 'errors.codes.forbidden',
  csrf_failed: 'errors.codes.csrf_failed',
  not_found: 'errors.codes.not_found',
  conflict: 'errors.codes.conflict',
  rate_limited: 'errors.codes.rate_limited',
  unsupported_media_type: 'errors.codes.unsupported_media_type',
  internal: 'errors.codes.internal',
  bad_request: 'errors.codes.bad_request',
  method_not_allowed: 'errors.codes.method_not_allowed',
  payload_too_large: 'errors.codes.payload_too_large',
  timeout: 'errors.codes.timeout',
  unavailable: 'errors.codes.unavailable',
  invalid_credentials: 'errors.codes.invalid_credentials',
  account_disabled: 'errors.codes.account_disabled',
  account_unverified: 'errors.codes.account_unverified',
  invalid_code: 'errors.codes.invalid_code',
  invalid_challenge: 'errors.codes.invalid_challenge',
  wrong_password: 'errors.codes.wrong_password',
  insecure_transport: 'errors.codes.insecure_transport',
  setup_required: 'errors.codes.setup_required',
  invalid_setup_token: 'errors.codes.invalid_setup_token',
  setup_completed: 'errors.codes.setup_completed',
  oidc_failed: 'errors.codes.oidc_failed',
  owner_required: 'errors.codes.owner_required',
  last_owner: 'errors.codes.last_owner',
  self_delete: 'errors.codes.self_delete',
  managed_by_config: 'errors.codes.managed_by_config',
  not_local_account: 'errors.codes.not_local_account',
}

// For responses without a (known) code: a proxy error page, a future code.
const STATUS_CODES: Readonly<Record<number, ErrorCode>> = {
  400: 'bad_request',
  401: 'unauthorized',
  403: 'forbidden',
  404: 'not_found',
  405: 'method_not_allowed',
  409: 'conflict',
  413: 'payload_too_large',
  415: 'unsupported_media_type',
  422: 'validation_failed',
  429: 'rate_limited',
  500: 'internal',
  502: 'unavailable',
  503: 'unavailable',
  504: 'timeout',
}

/** The error code an error stands for: its own, else one derived from the HTTP status. */
export function errorCodeOf(error: unknown): ErrorCode | undefined {
  if (isApiError(error) && error.code !== undefined) return error.code
  const status = httpStatusOf(error)
  return status === undefined ? undefined : STATUS_CODES[status]
}

export function errorCodeKey(code: ErrorCode): CodeKey {
  return CODE_KEYS[code]
}

/**
 * A user-facing, translated message for any error thrown by the API layer.
 * Raw messages are never shown: they are English and may contain internals.
 */
export function errorMessage(error: unknown, t: TFunction): string {
  if (isApiError(error) && error.retryAfterSeconds !== undefined && error.status === 429) {
    return t('errors.rateLimitedRetry', { seconds: error.retryAfterSeconds })
  }
  const code = errorCodeOf(error)
  if (code !== undefined) return t(CODE_KEYS[code])
  if (httpStatusOf(error) === undefined) return t('errors.network')
  return t('errors.unknown')
}
