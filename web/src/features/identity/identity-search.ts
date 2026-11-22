// URL state of the identity page: the tab (OIDC providers, or `tab=ldap` / `tab=tfa`).

export const IDENTITY_TABS = ['oidc', 'ldap', 'tfa'] as const

export type IdentityTab = (typeof IDENTITY_TABS)[number]

export interface IdentitySearch {
  /** Absent: the OIDC providers. */
  tab?: Exclude<IdentityTab, 'oidc'> | undefined
}

export function parseIdentitySearch(raw: Record<string, unknown>): IdentitySearch {
  return { tab: raw.tab === 'ldap' || raw.tab === 'tfa' ? raw.tab : undefined }
}
