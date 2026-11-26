import { isApiError } from '@/api/api-error'

export const LINK_REASONS = [
  'denied',
  'unavailable',
  'invalid_token',
  'identity_taken',
  'provider_linked',
  'directory_account',
  'account_inactive',
  'expired',
] as const

export type LinkReason = (typeof LINK_REASONS)[number]

export function linkReasonOf(error: unknown): LinkReason | undefined {
  if (!isApiError(error) || error.code !== 'oidc_failed') return undefined
  const reason = error.details?.reason
  return LINK_REASONS.find((known) => known === reason)
}
