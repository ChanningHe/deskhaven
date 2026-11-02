// The console's cookies are `__Host-` + `Secure`: on a plain-HTTP page the
// browser drops them, so signing in cannot work there. The server refuses such
// sign-ins with `insecure_transport`; the page also warns up front.

import { isApiError } from '@/api/api-error'

const LOOPBACK_HOSTS: ReadonlySet<string> = new Set(['localhost', '127.0.0.1', '[::1]'])

/**
 * Whether this page would lose `Secure` cookies: plain HTTP to anything but a
 * loopback host (browsers treat those as secure contexts).
 */
export function isInsecurePage(location: Pick<Location, 'protocol' | 'hostname'>): boolean {
  if (location.protocol !== 'http:') return false
  const host = location.hostname.toLowerCase()
  return !(LOOPBACK_HOSTS.has(host) || host.endsWith('.localhost') || host.startsWith('127.'))
}

export function isInsecureTransportError(error: unknown): boolean {
  return isApiError(error) && error.code === 'insecure_transport'
}
