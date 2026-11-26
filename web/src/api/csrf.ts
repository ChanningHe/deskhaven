// The server checks this header against the HttpOnly session cookie, not against this
// cookie, so reading it here grants nothing an attacker could forge.

export const CSRF_COOKIE = '__Host-deskhaven_csrf'
export const CSRF_HEADER = 'X-Deskhaven-CSRF'

/** Without a session any non-empty value passes; the header still forces a CORS preflight. */
export const ANONYMOUS_CSRF_TOKEN = 'anonymous'

const SAFE_METHODS: ReadonlySet<string> = new Set(['GET', 'HEAD', 'OPTIONS'])

export function needsCsrfHeader(method: string): boolean {
  return !SAFE_METHODS.has(method.toUpperCase())
}

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
