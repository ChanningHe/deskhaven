// `GET /api/version` is a client-API endpoint, not in the admin OpenAPI, so it is fetched by
// hand and validated here.

import { useQuery } from '@tanstack/react-query'

import { apiFetch } from '@/api/client'

const VERSION_URL = '/api/version'
const HTTPS_PROTOCOL = 'https:'

export interface ServerVersion {
  version: string
  /** Always an https URL; safe to use as an href. */
  sourceUrl: string
}

export function httpsUrlOf(value: string): string | undefined {
  try {
    const url = new URL(value)
    return url.protocol === HTTPS_PROTOCOL ? url.href : undefined
  } catch {
    return undefined
  }
}

export function parseServerVersion(body: unknown): ServerVersion | undefined {
  if (typeof body !== 'object' || body === null) return undefined
  const { version, source_url: sourceUrl } = body as Record<string, unknown>
  if (typeof version !== 'string' || version === '' || typeof sourceUrl !== 'string') {
    return undefined
  }
  const safeUrl = httpsUrlOf(sourceUrl)
  return safeUrl === undefined ? undefined : { version, sourceUrl: safeUrl }
}

/** A version cannot change without a restart: fetched once per page load, not retried. */
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
