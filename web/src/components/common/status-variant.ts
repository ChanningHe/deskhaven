import type { StatusView, TokenStatusView } from '@/api/generated/model'

/**
 * Badge colours carry one meaning everywhere: `success` usable, `secondary` stopped by someone,
 * `warning` needs attention, `destructive` failed, `outline` neutral or pending.
 */
export type StatusVariant = 'success' | 'secondary' | 'warning' | 'destructive' | 'outline'

const ACCOUNT: Record<StatusView, StatusVariant> = {
  active: 'success',
  disabled: 'secondary',
  unverified: 'outline',
}

const TOKEN: Record<TokenStatusView, StatusVariant> = {
  active: 'success',
  revoked: 'secondary',
  expired: 'secondary',
}

export const accountStatusVariant = (status: StatusView): StatusVariant => ACCOUNT[status]

export const tokenStatusVariant = (status: TokenStatusView): StatusVariant => TOKEN[status]

/** Login audit results are open-ended strings: anything but success or a lockout failed. */
export function loginResultVariant(result: string): StatusVariant {
  if (result === 'success') return 'success'
  if (result === 'locked') return 'warning'
  return 'destructive'
}

/** A fact that is either in effect or not: 2FA enabled, email verified, current session. */
export const flagVariant = (on: boolean): StatusVariant => (on ? 'success' : 'outline')
