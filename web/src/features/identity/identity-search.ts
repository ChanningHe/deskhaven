export const IDENTITY_TABS = ['oidc', 'ldap', 'tfa'] as const

export type IdentityTab = (typeof IDENTITY_TABS)[number]

export interface IdentitySearch {
  tab?: Exclude<IdentityTab, 'oidc'> | undefined
}

export function parseIdentitySearch(raw: Record<string, unknown>): IdentitySearch {
  return { tab: raw.tab === 'ldap' || raw.tab === 'tfa' ? raw.tab : undefined }
}
