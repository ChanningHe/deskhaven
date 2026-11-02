import { HttpError } from '@/lib/http-error'

import type { ErrorCode } from './generated/model'
import { ErrorCode as ERROR_CODES } from './generated/model'

/**
 * A non-2xx admin API response, parsed from the error envelope
 * `{error: {code, message, details?}}` (08 §6). `code` is undefined when the
 * body is not an envelope (a proxy error page, an unknown future code): callers
 * then fall back to the HTTP status.
 */
export class ApiError extends HttpError {
  override readonly name: string = 'ApiError'
  readonly code: ErrorCode | undefined
  readonly details: Readonly<Record<string, unknown>> | undefined
  /** Seconds to wait before retrying, from `Retry-After` (429, 503). */
  readonly retryAfterSeconds: number | undefined

  constructor(
    status: number,
    init: {
      code?: ErrorCode | undefined
      message?: string | undefined
      details?: Readonly<Record<string, unknown>> | undefined
      retryAfterSeconds?: number | undefined
    } = {},
  ) {
    super(status, init.message ?? `HTTP ${status}`)
    this.code = init.code
    this.details = init.details
    this.retryAfterSeconds = init.retryAfterSeconds
  }
}

const KNOWN_CODES: ReadonlySet<string> = new Set(Object.values(ERROR_CODES))

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function isErrorCode(value: unknown): value is ErrorCode {
  return typeof value === 'string' && KNOWN_CODES.has(value)
}

/** The envelope's fields, validated; anything malformed is dropped. */
export function parseErrorEnvelope(body: unknown): {
  code?: ErrorCode
  message?: string
  details?: Record<string, unknown>
} {
  if (!isRecord(body) || !isRecord(body.error)) return {}
  const { code, message, details } = body.error
  return {
    ...(isErrorCode(code) ? { code } : {}),
    ...(typeof message === 'string' ? { message } : {}),
    ...(isRecord(details) ? { details } : {}),
  }
}

const MS_PER_SECOND = 1000

/**
 * `Retry-After` as whole seconds (at least 1): either delta-seconds or an
 * HTTP date. Undefined when absent or unparseable.
 */
export function parseRetryAfter(value: string | null, now = Date.now()): number | undefined {
  if (value === null || value.trim() === '') return undefined
  const trimmed = value.trim()
  if (/^\d+$/.test(trimmed)) return Math.max(1, Number.parseInt(trimmed, 10))
  // An HTTP date always names the weekday and month; this rejects "-5" and the like,
  // which Date.parse would read as a year.
  if (!/[a-z]/i.test(trimmed)) return undefined
  const date = Date.parse(trimmed)
  if (Number.isNaN(date)) return undefined
  return Math.max(1, Math.ceil((date - now) / MS_PER_SECOND))
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError
}
