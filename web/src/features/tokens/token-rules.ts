// The server enforces every rule; these only decide what the console offers.

import type { ApiTokenView, Me, ScopeView } from '@/api/generated/model'

export const MS_PER_DAY = 86_400_000

export const EXPIRY_CHOICES = ['30', '90', '365', 'never'] as const
export type ExpiryChoice = (typeof EXPIRY_CHOICES)[number]

/** Preselected: a token should expire unless someone decides it must not. */
export const DEFAULT_EXPIRY: ExpiryChoice = '90'

export const NAME_MAX_LENGTH = 64

export const SCOPES: readonly ScopeView[] = ['deploy', 'assign', 'admin:read', 'admin:write']

/** The i18n key of a scope's description (a key cannot contain the `:` of a scope name). */
export const SCOPE_KEYS = {
  deploy: 'deploy',
  assign: 'assign',
  'admin:read': 'adminRead',
  'admin:write': 'adminWrite',
} as const satisfies Record<ScopeView, string>

export function expiryFromChoice(choice: ExpiryChoice, nowMs: number): number | null {
  return choice === 'never' ? null : nowMs + Number(choice) * MS_PER_DAY
}

export function canRevoke(me: Pick<Me, 'role'>, token: ApiTokenView): boolean {
  if (token.status !== 'active') return false
  return me.role === 'owner' || !isOwnerLevel(token)
}

export function isOwnerLevel(token: ApiTokenView): boolean {
  return token.created_by?.role !== 'admin'
}
