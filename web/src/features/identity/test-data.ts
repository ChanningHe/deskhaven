// Fixtures of the identity page tests.

import type {
  LdapDiagnosticView,
  LdapDirectoryView,
  OidcProviderView,
  SettingView,
} from '@/api/generated/model'

export const PROVIDERS_PATH = '/admin/api/v1/oidc-providers'
export const DIRECTORIES_PATH = '/admin/api/v1/ldap-directories'
export const CONNECTION_TEST_PATH = '/admin/api/v1/ldap-tests/connection'
export const USER_TEST_PATH = '/admin/api/v1/ldap-tests/user'
export const SETTINGS_PATH = '/admin/api/v1/settings'
export const CALLBACK_URL = 'https://rd.example.com/api/oidc/callback'

export function providerView(
  id: number,
  name: string,
  patch: Partial<OidcProviderView> = {},
): OidcProviderView {
  return {
    id,
    name,
    display_name: `${name} SSO`,
    icon: null,
    issuer: `https://${name}.example.com`,
    client_id: 'deskhaven',
    has_client_secret: true,
    scopes: 'openid profile email',
    auto_provision: false,
    group_claim: null,
    admin_group: null,
    enabled: true,
    managed_by: 'ui',
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

export function directoryView(patch: Partial<LdapDirectoryView> = {}): LdapDirectoryView {
  return {
    id: 4,
    name: 'corp',
    url: 'ldaps://ldap.example.org',
    start_tls: false,
    tls_verify: true,
    bind_dn: 'cn=svc,dc=example,dc=org',
    has_bind_password: true,
    user_base_dn: 'ou=people,dc=example,dc=org',
    user_filter: '(objectClass=person)',
    username_attr: 'uid',
    email_attr: 'mail',
    display_name_attr: 'cn',
    group_base_dn: null,
    group_filter: null,
    admin_group_dn: null,
    enabled: true,
    managed_by: 'ui',
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

export const USER_DIAGNOSTIC: LdapDiagnosticView = {
  passed: false,
  steps: [
    { step: 'connect', status: 'passed', message: 'connected to ldaps://ldap.example.org (TLS)' },
    { step: 'service_bind', status: 'passed', message: 'bound as cn=svc,dc=example,dc=org' },
    { step: 'user_search', status: 'passed', message: 'found uid=alice,ou=people' },
    { step: 'user_bind', status: 'failed', message: 'the directory refused the password' },
    { step: 'group_mapping', status: 'skipped', message: 'skipped after an earlier failure' },
  ],
  user: {
    dn: 'uid=alice,ou=people',
    username: 'alice',
    email: 'alice@example.org',
    display_name: 'Alice',
    directory_groups: [],
    mapped_groups: [],
    admin: null,
  },
}

export function tfaSetting(patch: Partial<SettingView> = {}): SettingView {
  return {
    key: 'auth.tfa.enforce',
    category: 'auth',
    description: 'Who must use two-factor authentication.',
    value: 'off',
    default: 'off',
    sensitive: false,
    pinned: false,
    version: 3,
    schema: { type: 'enum', variants: ['off', 'admins', 'all'] },
    source: { kind: 'default' },
    ...patch,
  }
}
