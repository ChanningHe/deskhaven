import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { FieldGroup } from '@/components/common/field-group'
import { TextField } from '@/features/auth/components/text-field'

import type { TranslationResources } from '@/i18n/resources'

import type { DirectoryDraft } from '../ldap-form'
import { CheckField, Warning } from './form-controls'

type TextKey = {
  [K in keyof DirectoryDraft]: DirectoryDraft[K] extends string ? K : never
}[keyof DirectoryDraft]

type FieldLabel = keyof TranslationResources['identity']['ldap']['fields']
type GroupTitle = keyof TranslationResources['identity']['ldap']['groups']
type CheckKey = 'enabled' | 'start_tls' | 'tls_verify'

/** The search and mapping groups in form order: draft key, label, optional hint. */
const SEARCH_GROUPS: readonly (readonly [
  GroupTitle,
  readonly (readonly [TextKey, FieldLabel, FieldLabel?])[],
])[] = [
  [
    'userLookup',
    [
      ['user_base_dn', 'userBaseDn'],
      ['user_filter', 'userFilter', 'userFilterHint'],
      ['username_attr', 'usernameAttr', 'usernameAttrHint'],
    ],
  ],
  [
    'attributes',
    [
      ['email_attr', 'emailAttr'],
      ['display_name_attr', 'displayNameAttr'],
    ],
  ],
  [
    'groupMapping',
    [
      ['group_base_dn', 'groupBaseDn', 'groupBaseDnHint'],
      ['group_filter', 'groupFilter', 'groupFilterHint'],
      ['admin_group_dn', 'adminGroupDn', 'adminGroupDnHint'],
    ],
  ],
]

interface DirectoryFieldsProps {
  draft: DirectoryDraft
  set: (change: Partial<DirectoryDraft>) => void
  /** A stored directory: whether it has a bind password (never shown). */
  stored: { hasPassword: boolean } | undefined
  /** The edit moves the stored password: it must be typed again. */
  needsPassword: boolean
  disabled: boolean
}

/** The inputs of the LDAP directory form, in five titled groups. */
export function DirectoryFields(props: DirectoryFieldsProps) {
  const { draft, set, disabled } = props
  const { t } = useTranslation()
  const id = useId()
  const text = (key: TextKey, label: FieldLabel, hint?: FieldLabel) => (
    <TextField
      key={key}
      id={`${id}-${key}`}
      label={t(`identity.ldap.fields.${label}`)}
      hint={hint === undefined ? undefined : t(`identity.ldap.fields.${hint}`)}
      value={draft[key]}
      disabled={disabled}
      onChange={(event) => {
        set({ [key]: event.target.value })
      }}
    />
  )
  const check = (key: CheckKey, label: FieldLabel) => (
    <CheckField
      label={t(`identity.ldap.fields.${label}`)}
      checked={draft[key]}
      disabled={disabled}
      onChange={(checked) => {
        set({ [key]: checked })
      }}
    />
  )
  return (
    <div className="grid gap-6">
      <FieldGroup title={t('identity.ldap.groups.connection')}>
        {check('enabled', 'enabled')}
        {text('name', 'name')}
        {text('url', 'url', 'urlHint')}
        {check('start_tls', 'startTls')}
        {check('tls_verify', 'tlsVerify')}
        {draft.tls_verify ? null : (
          <Warning tone="destructive">{t('identity.ldap.fields.tlsVerifyWarning')}</Warning>
        )}
      </FieldGroup>
      <FieldGroup title={t('identity.ldap.groups.serviceAccount')}>
        {text('bind_dn', 'bindDn', 'bindDnHint')}
        <PasswordField {...props} id={`${id}-bind_password`} />
      </FieldGroup>
      {SEARCH_GROUPS.map(([title, fields]) => (
        <FieldGroup key={title} title={t(`identity.ldap.groups.${title}`)}>
          {fields.map(([key, label, hint]) => text(key, label, hint))}
        </FieldGroup>
      ))}
    </div>
  )
}

function PasswordField({
  id,
  draft,
  set,
  stored,
  needsPassword,
  disabled,
}: DirectoryFieldsProps & { id: string }) {
  const { t } = useTranslation()
  const hint = needsPassword
    ? t('identity.ldap.fields.reenterPassword')
    : stored?.hasPassword === true
      ? t('identity.ldap.fields.bindPasswordSet')
      : undefined
  return (
    <TextField
      id={id}
      label={t('identity.ldap.fields.bindPassword')}
      hint={hint}
      type="password"
      autoComplete="new-password"
      value={draft.bind_password}
      disabled={disabled}
      aria-invalid={needsPassword || undefined}
      onChange={(event) => {
        set({ bind_password: event.target.value })
      }}
    />
  )
}
