// What the token page decides locally. The server enforces every rule (06 §4.1); these
// only decide what the console offers.

import type { ApiTokenView, Me, ScopeView } from '@/api/generated/model'

export const MS_PER_DAY = 86_400_000

/** What a new token's expiry offers: a number of days, or no expiry. */
export const EXPIRY_CHOICES = ['30', '90', '365', 'never'] as const
export type ExpiryChoice = (typeof EXPIRY_CHOICES)[number]

/** Preselected: a token should expire unless someone decides it must not. */
export const DEFAULT_EXPIRY: ExpiryChoice = '90'

/** Longest token name the API accepts. */
export const NAME_MAX_LENGTH = 64

/** The scopes a token can hold, in the order the API lists them. */
export const SCOPES: readonly ScopeView[] = ['deploy', 'assign', 'admin:read', 'admin:write']

/** The i18n key of a scope's description (a key cannot contain the `:` of a scope name). */
export const SCOPE_KEYS = {
  deploy: 'deploy',
  assign: 'assign',
  'admin:read': 'adminRead',
  'admin:write': 'adminWrite',
} as const satisfies Record<ScopeView, string>

/** The unix-millisecond expiry for `choice` chosen at `nowMs`; `null`: never expires. */
export function expiryFromChoice(choice: ExpiryChoice, nowMs: number): number | null {
  return choice === 'never' ? null : nowMs + Number(choice) * MS_PER_DAY
}

/**
 * Whether `me` is offered "revoke" on `token`: only a token that still works, and a token
 * of an owner (or one without creator) only to an owner.
 */
export function canRevoke(me: Pick<Me, 'role'>, token: ApiTokenView): boolean {
  if (token.status !== 'active') return false
  return me.role === 'owner' || !isOwnerLevel(token)
}

/** Whether the token is an owner's or a system token, which an admin may not revoke. */
export function isOwnerLevel(token: ApiTokenView): boolean {
  return token.created_by?.role !== 'admin'
}
