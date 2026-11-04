import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { TextField } from '@/features/auth/components/text-field'

import type { TranslationResources } from '@/i18n/resources'

import type { DirectoryDraft } from '../ldap-form'
import { CheckField, Warning } from './form-controls'

type TextKey = {
  [K in keyof DirectoryDraft]: DirectoryDraft[K] extends string ? K : never
}[keyof DirectoryDraft]

type FieldLabel = keyof TranslationResources['identity']['ldap']['fields']

/** Search and mapping fields in form order: draft key, label, optional hint. */
const SEARCH_FIELDS: readonly (readonly [TextKey, FieldLabel, FieldLabel?])[] = [
  ['user_base_dn', 'userBaseDn'],
  ['user_filter', 'userFilter', 'userFilterHint'],
  ['username_attr', 'usernameAttr', 'usernameAttrHint'],
  ['email_attr', 'emailAttr'],
  ['display_name_attr', 'displayNameAttr'],
  ['group_base_dn', 'groupBaseDn', 'groupBaseDnHint'],
  ['group_filter', 'groupFilter', 'groupFilterHint'],
  ['admin_group_dn', 'adminGroupDn', 'adminGroupDnHint'],
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

/** The inputs of the LDAP directory form. */
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
  return (
    <div className="grid gap-3">
      {text('name', 'name')}
      {text('url', 'url', 'urlHint')}
      <CheckField
        label={t('identity.ldap.fields.startTls')}
        checked={draft.start_tls}
        disabled={disabled}
        onChange={(checked) => {
          set({ start_tls: checked })
        }}
      />
      <CheckField
        label={t('identity.ldap.fields.tlsVerify')}
        checked={draft.tls_verify}
        disabled={disabled}
        onChange={(checked) => {
          set({ tls_verify: checked })
        }}
      />
      {draft.tls_verify ? null : (
        <Warning tone="destructive">{t('identity.ldap.fields.tlsVerifyWarning')}</Warning>
      )}
      {text('bind_dn', 'bindDn', 'bindDnHint')}
      <PasswordField {...props} id={`${id}-bind_password`} />
      {SEARCH_FIELDS.map(([key, label, hint]) => text(key, label, hint))}
      <CheckField
        label={t('identity.ldap.fields.enabled')}
        checked={draft.enabled}
        disabled={disabled}
        onChange={(checked) => {
          set({ enabled: checked })
        }}
      />
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
