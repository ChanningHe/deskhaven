// The one fetch wrapper every admin API call goes through; orval's generated
// client uses it as its mutator (orval.config.ts). It
// - sends cookies same-origin only and asks for JSON,
// - adds the CSRF header to state-changing requests,
// - throws ApiError (a HttpError) for non-2xx responses, parsed from the error
//   envelope, with Retry-After surfaced as `retryAfterSeconds`.
// Network failures reject with the browser's TypeError (no status): the query
// policy retries those once. 401 handling (redirect to /login) lives in the
// query client, so it also covers requests made outside React Query.

import { ApiError, parseErrorEnvelope, parseRetryAfter } from './api-error'
import { ANONYMOUS_CSRF_TOKEN, CSRF_HEADER, needsCsrfHeader, readCsrfToken } from './csrf'

const HTTP_NO_CONTENT = 204

function isJson(response: Response): boolean {
  return response.headers.get('Content-Type')?.includes('application/json') ?? false
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return undefined
  }
}

async function toApiError(response: Response): Promise<ApiError> {
  const body = isJson(response) ? await readJson(response) : undefined
  return new ApiError(response.status, {
    ...parseErrorEnvelope(body),
    retryAfterSeconds: parseRetryAfter(response.headers.get('Retry-After')),
  })
}

function requestHeaders(init: RequestInit, method: string): Headers {
  const headers = new Headers(init.headers)
  headers.set('Accept', 'application/json')
  // Only JSON text bodies: FormData / Blob / URLSearchParams carry their own type
  // (multipart boundary), which the browser sets.
  if (typeof init.body === 'string' && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  if (needsCsrfHeader(method)) {
    headers.set(CSRF_HEADER, readCsrfToken(document.cookie) ?? ANONYMOUS_CSRF_TOKEN)
  }
  return headers
}

/**
 * Performs an admin API request and returns the parsed JSON body. The body is
 * typed by the caller (the generated client, from openapi.json); it comes from
 * our own same-origin server and is not validated at runtime.
 */
export async function apiFetch<T>(url: string, init: RequestInit = {}): Promise<T> {
  const method = (init.method ?? 'GET').toUpperCase()
  const response = await fetch(url, {
    ...init,
    method,
    headers: requestHeaders(init, method),
    credentials: 'same-origin',
  })
  if (!response.ok) throw await toApiError(response)
  if (response.status === HTTP_NO_CONTENT) return undefined as T
  try {
    return (await response.json()) as T
  } catch {
    // A success without the expected JSON (a proxy page, a truncated body) is a
    // server fault with a status, not a network error: no retry, no "offline".
    throw new ApiError(response.status, { message: 'response is not valid JSON' })
  }
}

/**
 * Error type of the generated hooks (orval reads this export from the mutator):
 * apiFetch never rejects with the raw envelope, only with ApiError or a network
 * error. Orval passes the documented error body as the type argument.
 */
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the signature is orval's contract
export type ErrorType<_Envelope> = ApiError | Error
