// The LDAP directory form: its draft and the request bodies built from it. The bind
// password is write-only: the draft starts empty and only a typed password is sent.

import type {
  LdapDirectoryPatch,
  LdapDirectoryView,
  NewLdapDirectoryBody,
} from '@/api/generated/model'

export interface DirectoryDraft {
  name: string
  url: string
  start_tls: boolean
  tls_verify: boolean
  bind_dn: string
  /** A new password; empty keeps the stored one. */
  bind_password: string
  user_base_dn: string
  user_filter: string
  username_attr: string
  email_attr: string
  display_name_attr: string
  group_base_dn: string
  group_filter: string
  admin_group_dn: string
  enabled: boolean
}

export const EMPTY_DIRECTORY: DirectoryDraft = {
  name: '',
  url: 'ldaps://',
  start_tls: false,
  tls_verify: true,
  bind_dn: '',
  bind_password: '',
  user_base_dn: '',
  user_filter: '',
  username_attr: '',
  email_attr: 'mail',
  display_name_attr: 'cn',
  group_base_dn: '',
  group_filter: '',
  admin_group_dn: '',
  enabled: true,
}

/** Fields that decide where the bind password goes: changing one needs it again. */
const CONNECTION_FIELDS = ['url', 'start_tls', 'tls_verify', 'bind_dn'] as const

export function directoryDraft(directory: LdapDirectoryView): DirectoryDraft {
  return {
    name: directory.name,
    url: directory.url,
    start_tls: directory.start_tls,
    tls_verify: directory.tls_verify,
    bind_dn: directory.bind_dn ?? '',
    bind_password: '',
    user_base_dn: directory.user_base_dn,
    user_filter: directory.user_filter,
    username_attr: directory.username_attr,
    email_attr: directory.email_attr ?? '',
    display_name_attr: directory.display_name_attr ?? '',
    group_base_dn: directory.group_base_dn ?? '',
    group_filter: directory.group_filter ?? '',
    admin_group_dn: directory.admin_group_dn ?? '',
    enabled: directory.enabled,
  }
}

function optional(value: string): string | null {
  const trimmed = value.trim()
  return trimmed === '' ? null : trimmed
}

/** The whole form, to create a directory or to test it unsaved. */
export function newDirectoryBody(draft: DirectoryDraft): NewLdapDirectoryBody {
  return {
    name: draft.name.trim(),
    url: draft.url.trim(),
    start_tls: draft.start_tls,
    tls_verify: draft.tls_verify,
    bind_dn: optional(draft.bind_dn),
    bind_password: draft.bind_password === '' ? null : draft.bind_password,
    user_base_dn: draft.user_base_dn.trim(),
    user_filter: draft.user_filter.trim(),
    username_attr: draft.username_attr.trim(),
    email_attr: optional(draft.email_attr),
    display_name_attr: optional(draft.display_name_attr),
    group_base_dn: optional(draft.group_base_dn),
    group_filter: optional(draft.group_filter),
    admin_group_dn: optional(draft.admin_group_dn),
    enabled: draft.enabled,
  }
}

/** Only the edited fields, and the password when one was typed. */
export function directoryPatch(
  draft: DirectoryDraft,
  changed: readonly (keyof DirectoryDraft)[],
): LdapDirectoryPatch {
  const full = newDirectoryBody(draft)
  const edited = changed.filter((key) => key !== 'bind_password')
  const fields = Object.fromEntries(edited.map((key) => [key, full[key] ?? null]))
  const password = draft.bind_password === '' ? {} : { bind_password: draft.bind_password }
  return { ...fields, ...password }
}

/** The edit moves the stored password elsewhere without it being entered again. */
export function needsPassword(
  draft: DirectoryDraft,
  changed: readonly (keyof DirectoryDraft)[],
): boolean {
  const moved = CONNECTION_FIELDS.some((key) => changed.includes(key))
  return moved && draft.bind_dn.trim() !== '' && draft.bind_password === ''
}
