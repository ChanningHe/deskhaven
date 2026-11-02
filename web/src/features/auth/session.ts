// The console session as seen by the SPA: `GET /admin/api/v1/me`. The session
// cookie is HttpOnly, so this request is the only way to learn whether we are
// signed in and with which role.

import type { QueryClient } from '@tanstack/react-query'
import { queryOptions, useSuspenseQuery } from '@tanstack/react-query'
import { redirect } from '@tanstack/react-router'

import { getGetMeQueryKey, getMe } from '@/api/generated/me/me'
import type { Me } from '@/api/generated/model'
import { HTTP_UNAUTHORIZED, httpStatusOf } from '@/lib/http-error'

export const ME_QUERY_KEY = getGetMeQueryKey()

// The role rarely changes; a background refetch every few minutes is plenty.
const ME_STALE_TIME_MS = 5 * 60_000

export function meQueryOptions() {
  return queryOptions({
    queryKey: ME_QUERY_KEY,
    queryFn: ({ signal }) => getMe({ signal }),
    staleTime: ME_STALE_TIME_MS,
  })
}

/** The signed-in user. Only valid below the `_app` layout, whose guard loaded it. */
export function useMe(): Me {
  return useSuspenseQuery(meQueryOptions()).data
}

export const LOGIN_PATH = '/login'

const DEL = 0x7f
const FIRST_PRINTABLE = 0x20

/**
 * Control characters and backslashes never belong in a redirect: URL parsers
 * strip TAB/CR/LF and read `\` as `/`, which turns `/\t/evil` or `/\evil` into
 * a protocol-relative URL to another host.
 */
function hasUnsafeCharacter(value: string): boolean {
  for (const char of value) {
    const code = char.charCodeAt(0)
    if (code < FIRST_PRINTABLE || code === DEL || char === '\\') return true
  }
  return false
}

/**
 * Where the login page may send the user back to: a path on this origin
 * (checked by actually resolving it, not by prefix), never the login page
 * itself. Returns the normalised `path?query#hash`.
 */
export function safeRedirectTarget(
  value: unknown,
  origin: string = window.location.origin,
): string | undefined {
  if (typeof value !== 'string' || !value.startsWith('/') || hasUnsafeCharacter(value)) {
    return undefined
  }
  let url: URL
  try {
    url = new URL(value, origin)
  } catch {
    return undefined
  }
  if (url.origin !== origin || url.pathname === LOGIN_PATH) return undefined
  return `${url.pathname}${url.search}${url.hash}`
}

/** A router redirect to the login page that comes back to `href` afterwards. */
export function loginRedirect(href: string) {
  const target = safeRedirectTarget(href)
  return redirect({ to: LOGIN_PATH, search: target === undefined ? {} : { redirect: target } })
}

/**
 * Route guard for the console layout: resolves the session (cached), or
 * redirects to the login page when there is none (401). Other failures
 * (network, 5xx) propagate to the route error component, which offers a retry.
 */
export async function requireSession(queryClient: QueryClient, href: string): Promise<Me> {
  try {
    // Cached while fresh; a stale session is re-checked before the navigation completes.
    return await queryClient.query(meQueryOptions())
  } catch (error) {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- router control flow
    if (httpStatusOf(error) === HTTP_UNAUTHORIZED) throw loginRedirect(href)
    throw error
  }
}
