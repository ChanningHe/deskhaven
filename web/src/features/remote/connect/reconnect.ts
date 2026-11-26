import { errorCodeOf } from '@/api/error-message'
import type { ErrorCode } from '@/api/generated/model'

import type { CloseBy } from '../handshake/handshake'
import type { ConnectFailure } from './controller'

// 27 s ± 20 %; the peer keeps a lost session for 30 s.
export const RETRY_DELAYS_MS: readonly number[] = [1000, 2000, 4000, 8000, 12_000]

export const RECENT_MS = 30_000
// Our last message heard only approximates the peer's last receive time.
export const RECENT_MARGIN_MS = 5000
const JITTER = 0.2

export const STEADY_MS = 30_000

const RECONNECTS: Readonly<Record<CloseBy, boolean>> = {
  user: false,
  peer: false,
  link: true,
  local: false,
  session_switch: true,
  restart: false,
}

export function reconnects(by: CloseBy): boolean {
  return RECONNECTS[by]
}

export function retryDelay(attempt: number, random: () => number): number | null {
  const base = RETRY_DELAYS_MS[attempt - 1]
  if (base === undefined) return null
  return Math.round(base * (1 + JITTER * (2 * random() - 1)))
}

// Retries only fill the server's rate limit up.
const FINAL_API_ERRORS = new Set<ErrorCode | undefined>(['rate_limited', 'too_many_sessions'])

export function retriesFailure(failure: ConnectFailure): boolean {
  if (failure.kind === 'no_decoder') return false
  if (failure.kind === 'api') return !FINAL_API_ERRORS.has(errorCodeOf(failure.error))
  return true
}
