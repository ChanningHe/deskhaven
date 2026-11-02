// CSRF double submit (06 §6): every state-changing admin API request carries the
// `X-Deskhaven-CSRF` header with the value of the JS-readable CSRF cookie. The
// server checks the header against the HttpOnly session cookie, not against this
// cookie, so reading it here grants nothing an attacker could forge.

export const CSRF_COOKIE = '__Host-deskhaven_csrf'
export const CSRF_HEADER = 'X-Deskhaven-CSRF'

/**
 * Sent when there is no CSRF cookie (no session yet, e.g. the login request).
 * The server accepts any non-empty value without a session; the header itself
 * still forces a CORS preflight, which cross-site pages never pass.
 */
export const ANONYMOUS_CSRF_TOKEN = 'anonymous'

const SAFE_METHODS: ReadonlySet<string> = new Set(['GET', 'HEAD', 'OPTIONS'])

export function needsCsrfHeader(method: string): boolean {
  return !SAFE_METHODS.has(method.toUpperCase())
}

/** The CSRF cookie's value from a `document.cookie` string, if present and non-empty. */
export function readCsrfToken(cookieHeader: string): string | undefined {
  for (const pair of cookieHeader.split(';')) {
    const separator = pair.indexOf('=')
    if (separator < 0) continue
    if (pair.slice(0, separator).trim() !== CSRF_COOKIE) continue
    const value = pair.slice(separator + 1).trim()
    return value === '' ? undefined : value
  }
  return undefined
}
