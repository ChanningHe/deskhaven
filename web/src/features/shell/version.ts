// The running server's version and where its source lives (AGPL-3.0 section 13).
// `GET /api/version` is the public client-API endpoint, not part of the admin
// OpenAPI, so it is fetched by hand and validated here.

import { useQuery } from '@tanstack/react-query'

import { apiFetch } from '@/api/client'

const VERSION_URL = '/api/version'
const HTTPS_PROTOCOL = 'https:'

export interface ServerVersion {
  version: string
  /** Always an https URL; safe to use as an href. */
  sourceUrl: string
}

/** The https URL as normalised by the URL parser, or undefined for anything else. */
function httpsUrlOf(value: string): string | undefined {
  try {
    const url = new URL(value)
    return url.protocol === HTTPS_PROTOCOL ? url.href : undefined
  } catch {
    return undefined
  }
}

/** Accepts only a body whose two fields are strings and whose source is https. */
export function parseServerVersion(body: unknown): ServerVersion | undefined {
  if (typeof body !== 'object' || body === null) return undefined
  const { version, source_url: sourceUrl } = body as Record<string, unknown>
  if (typeof version !== 'string' || version === '' || typeof sourceUrl !== 'string') {
    return undefined
  }
  const safeUrl = httpsUrlOf(sourceUrl)
  return safeUrl === undefined ? undefined : { version, sourceUrl: safeUrl }
}

/**
 * The server version, or undefined while loading and on any failure: this is a
 * footnote, never worth an error state. A version cannot change without a
 * restart, so it is fetched once per page load and not retried.
 */
export function useServerVersion(): ServerVersion | undefined {
  const { data } = useQuery({
    queryKey: ['server', 'version'],
    queryFn: async ({ signal }) =>
      parseServerVersion(await apiFetch<unknown>(VERSION_URL, { signal })) ?? null,
    staleTime: Infinity,
    retry: false,
  })
  return data ?? undefined
}
