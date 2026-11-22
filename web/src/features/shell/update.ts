// A newer deskhaven release than the running server, as the server learnt it
// from GitHub. The server builds the release link itself; the console still
// accepts only an https one before using it as an href.

import { useGetUpdateCheck } from '@/api/generated/update-check/update-check'
import type { UpdateCheckView } from '@/api/generated/model'

import { httpsUrlOf } from './version'

const UPDATE_STALE_TIME_MS = 60 * 60_000

export interface AvailableUpdate {
  version: string
  /** Always an https URL; safe to use as an href. */
  releaseUrl: string
}

function parseUpdate({
  latest,
  release_url: releaseUrl,
}: UpdateCheckView): AvailableUpdate | undefined {
  if (typeof latest !== 'string' || latest === '' || typeof releaseUrl !== 'string') {
    return undefined
  }
  const safeUrl = httpsUrlOf(releaseUrl)
  return safeUrl === undefined ? undefined : { version: latest, releaseUrl: safeUrl }
}

/**
 * The newer release, or undefined while loading, when there is none and on any
 * failure: a footnote, never worth an error state. `enabled: false` (users
 * without admin rights) sends no request.
 */
export function useAvailableUpdate(enabled: boolean): AvailableUpdate | undefined {
  const { data } = useGetUpdateCheck({
    query: { enabled, staleTime: UPDATE_STALE_TIME_MS, retry: false, select: parseUpdate },
  })
  return data
}
