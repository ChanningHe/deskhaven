import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  useBeginTotpEnrolment,
  useConfirmTotpEnrolment,
  useDisableTotp,
  useRegenerateRecoveryCodes,
} from '@/api/generated/me/me'
import type { MyAccount } from '@/api/generated/model'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'

import { refreshAccount } from '../invalidate'
import { EnrolDialog } from './enrol-dialog'
import { ReauthDialog } from './reauth-dialog'
import { Section } from './section'

type Dialog = 'enrol' | 'disable' | 'regenerate'

/**
 * TOTP: enrol (QR, password and code, recovery codes once), turn off (password; refused
 * while `auth.tfa.enforce` requires it) and regenerate the recovery codes (password).
 * Single sign-on accounts have no local second factor.
 */
export function TwoFactorCard({ account }: { account: MyAccount }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  // gcTime 0: seeds and recovery codes are not kept in the cache once shown.
  const begin = useBeginTotpEnrolment(SECRET_MUTATION)
  const confirm = useConfirmTotpEnrolment(SECRET_MUTATION)
  const regenerate = useRegenerateRecoveryCodes(SECRET_MUTATION)
  const disable = useDisableTotp()
  const [dialog, setDialog] = useState<Dialog | undefined>(undefined)
  const codes = confirm.data?.recovery_codes ?? regenerate.data?.recovery_codes

  const startEnrolment = () => {
    confirm.reset()
    begin.mutate()
    setDialog('enrol')
  }
  const closeEnrolment = () => {
    setDialog(undefined)
    begin.reset()
  }

  return (
    <Section title={t('me.tfa.title')} description={t('me.tfa.description')}>
      {account.can_use_totp ? (
        <div className="space-y-3">
          <p className="flex flex-wrap items-center gap-2 text-sm">
            <Badge variant={account.totp_enabled ? 'secondary' : 'outline'}>
              {account.totp_enabled ? t('me.tfa.on') : t('me.tfa.off')}
            </Badge>
            {account.totp_enabled ? (
              <span className="text-muted-foreground">
                {t('me.tfa.recoveryLeft', { count: account.recovery_codes_left })}
              </span>
            ) : null}
          </p>
          {account.totp_enabled ? (
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setDialog('regenerate')
                }}
              >
                {t('me.tfa.regenerate')}
              </Button>
              <Button
                size="sm"
                variant="destructive"
                disabled={account.tfa_required}
                onClick={() => {
                  setDialog('disable')
                }}
              >
                {t('me.tfa.disable')}
              </Button>
            </div>
          ) : (
            <Button size="sm" onClick={startEnrolment}>
              {t('me.tfa.enable')}
            </Button>
          )}
          {account.tfa_required ? (
            <p className="text-sm text-muted-foreground">{t('me.tfa.requiredHint')}</p>
          ) : null}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t('me.tfa.external')}</p>
      )}
      <EnrolDialog
        open={dialog === 'enrol'}
        onOpenChange={(open) => {
          if (!open) closeEnrolment()
        }}
        enrolment={begin.data}
        beginError={begin.error}
        onRetry={() => {
          begin.mutate()
        }}
        onConfirm={async (data) => {
          const enabled = await confirm.mutateAsync({ data })
          closeEnrolment()
          await refreshAccount(queryClient, enabled.user)
        }}
      />
      <ReauthDialog
        open={dialog === 'disable'}
        onOpenChange={(open) => {
          if (!open) setDialog(undefined)
        }}
        title={t('me.tfa.disableTitle')}
        description={t('me.tfa.disableDescription')}
        submitLabel={t('me.tfa.disableSubmit')}
        onSubmit={async (currentPassword, code) => {
          const data = { current_password: currentPassword, code }
          const me = await disable.mutateAsync({ data })
          await refreshAccount(queryClient, me)
        }}
      />
      <ReauthDialog
        open={dialog === 'regenerate'}
        onOpenChange={(open) => {
          if (!open) setDialog(undefined)
        }}
        title={t('me.tfa.regenerateTitle')}
        description={t('me.tfa.regenerateDescription')}
        submitLabel={t('me.tfa.regenerateSubmit')}
        onSubmit={async (currentPassword, code) => {
          await regenerate.mutateAsync({ data: { current_password: currentPassword, code } })
          await refreshAccount(queryClient)
        }}
      />
      <SecretRevealDialog
        open={codes !== undefined}
        title={t('accountSetup.recoveryTitle')}
        description={t('accountSetup.recoveryDescription')}
        secret={codes?.join('\n') ?? ''}
        onDone={() => {
          confirm.reset()
          regenerate.reset()
        }}
      />
    </Section>
  )
}
