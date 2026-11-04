import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { TextField } from '@/features/auth/components/text-field'

import type { ProviderDraft } from '../oidc-form'
import { CheckField, Warning } from './form-controls'

interface ProviderFieldsProps {
  draft: ProviderDraft
  set: (change: Partial<ProviderDraft>) => void
  /** Editing a stored provider: the issuer is fixed, the secret may be kept. */
  stored: { hasSecret: boolean } | undefined
  disabled: boolean
}

/** The inputs of the OIDC provider form. */
export function ProviderFields({ draft, set, stored, disabled }: ProviderFieldsProps) {
  const { t } = useTranslation()
  const id = useId()
  const text = (key: keyof ProviderDraft) => ({
    id: `${id}-${key}`,
    value: String(draft[key]),
    disabled,
    onChange: (event: { target: { value: string } }) => {
      set({ [key]: event.target.value })
    },
  })
  const secretHint = stored?.hasSecret === true ? 'secretSet' : 'secretNone'
  return (
    <div className="grid gap-3">
      <TextField
        {...text('name')}
        label={t('identity.oidc.fields.name')}
        hint={t('identity.oidc.fields.nameHint')}
        required
        maxLength={32}
      />
      <TextField {...text('display_name')} label={t('identity.oidc.fields.displayName')} />
      <TextField
        {...text('issuer')}
        label={t('identity.oidc.fields.issuer')}
        hint={t('identity.oidc.fields.issuerHint')}
        required
        type="url"
        disabled={disabled || stored !== undefined}
      />
      <TextField {...text('client_id')} label={t('identity.oidc.fields.clientId')} required />
      <TextField
        {...text('client_secret')}
        label={t('identity.oidc.fields.clientSecret')}
        hint={stored === undefined ? undefined : t(`identity.oidc.fields.${secretHint}`)}
        type="password"
        autoComplete="new-password"
        disabled={disabled || draft.clear_secret}
      />
      {stored?.hasSecret === true ? (
        <CheckField
          label={t('identity.oidc.fields.clearSecret')}
          checked={draft.clear_secret}
          disabled={disabled}
          onChange={(checked) => {
            set({ clear_secret: checked, client_secret: '' })
          }}
        />
      ) : null}
      <TextField {...text('scopes')} label={t('identity.oidc.fields.scopes')} />
      <CheckField
        label={t('identity.oidc.fields.autoProvision')}
        checked={draft.auto_provision}
        disabled={disabled}
        onChange={(checked) => {
          set({ auto_provision: checked })
        }}
      />
      <TextField
        {...text('group_claim')}
        label={t('identity.oidc.fields.groupClaim')}
        hint={t('identity.oidc.fields.groupClaimHint')}
      />
      <TextField {...text('admin_group')} label={t('identity.oidc.fields.adminGroup')} />
      <Warning>{t('identity.oidc.fields.adminGroupWarning')}</Warning>
      <CheckField
        label={t('identity.oidc.fields.enabled')}
        checked={draft.enabled}
        disabled={disabled}
        onChange={(checked) => {
          set({ enabled: checked })
        }}
      />
    </div>
  )
}
