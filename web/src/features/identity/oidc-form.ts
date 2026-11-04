// The OIDC provider form: its draft and the request bodies built from it. The client
// secret is write-only: the draft starts empty, and only a typed secret (or the explicit
// "public client" choice) is ever sent.

import type {
  NewOidcProviderBody,
  OidcProviderPatch,
  OidcProviderView,
} from '@/api/generated/model'

export interface ProviderDraft {
  name: string
  display_name: string
  issuer: string
  client_id: string
  /** A new secret; empty keeps the stored one. */
  client_secret: string
  /** Remove the stored secret (a public client). */
  clear_secret: boolean
  scopes: string
  auto_provision: boolean
  group_claim: string
  admin_group: string
  enabled: boolean
}

export const DEFAULT_SCOPES = 'openid profile email'

export const EMPTY_PROVIDER: ProviderDraft = {
  name: '',
  display_name: '',
  issuer: '',
  client_id: '',
  client_secret: '',
  clear_secret: false,
  scopes: DEFAULT_SCOPES,
  auto_provision: false,
  group_claim: '',
  admin_group: '',
  enabled: true,
}

export function providerDraft(provider: OidcProviderView): ProviderDraft {
  return {
    name: provider.name,
    display_name: provider.display_name,
    issuer: provider.issuer,
    client_id: provider.client_id,
    client_secret: '',
    clear_secret: false,
    scopes: provider.scopes,
    auto_provision: provider.auto_provision,
    group_claim: provider.group_claim ?? '',
    admin_group: provider.admin_group ?? '',
    enabled: provider.enabled,
  }
}

function optional(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

export function newProviderBody(draft: ProviderDraft): NewOidcProviderBody {
  return {
    name: draft.name.trim(),
    display_name: draft.display_name.trim(),
    issuer: draft.issuer.trim(),
    client_id: draft.client_id.trim(),
    client_secret: draft.client_secret === '' ? null : draft.client_secret,
    scopes: optional(draft.scopes),
    auto_provision: draft.auto_provision,
    group_claim: optional(draft.group_claim),
    admin_group: optional(draft.admin_group),
    enabled: draft.enabled,
  }
}

/** Only the edited fields: an untouched field must not write a stale value back. */
export function providerPatch(
  draft: ProviderDraft,
  changed: readonly (keyof ProviderDraft)[],
): OidcProviderPatch {
  const pick = <K extends keyof OidcProviderPatch>(
    key: K & keyof ProviderDraft,
    value: OidcProviderPatch[K],
  ): OidcProviderPatch => (changed.includes(key) ? { [key]: value } : {})
  return {
    ...pick('name', draft.name.trim()),
    ...pick('display_name', draft.display_name.trim()),
    ...pick('client_id', draft.client_id.trim()),
    ...pick('scopes', draft.scopes.trim()),
    ...pick('auto_provision', draft.auto_provision),
    ...pick('group_claim', optional(draft.group_claim)),
    ...pick('admin_group', optional(draft.admin_group)),
    ...pick('enabled', draft.enabled),
    ...secretChange(draft),
  }
}

function secretChange(draft: ProviderDraft): OidcProviderPatch {
  if (draft.clear_secret) return { client_secret: null }
  return draft.client_secret === '' ? {} : { client_secret: draft.client_secret }
}
