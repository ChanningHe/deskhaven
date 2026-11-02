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

/**
 * Where the login page may send the user back to: a same-origin path only
 * (no `//host`, no `/\host`, no scheme), and never the login page itself.
 */
export function safeRedirectTarget(value: unknown): string | undefined {
  if (typeof value !== 'string' || !value.startsWith('/')) return undefined
  if (value.startsWith('//') || value.startsWith('/\\')) return undefined
  if (value === LOGIN_PATH || value.startsWith(`${LOGIN_PATH}?`)) return undefined
  return value
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
