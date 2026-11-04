import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useChangePassword } from '@/api/generated/me/me'
import type { MyAccount } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { TextField } from '@/features/auth/components/text-field'

import { refreshAccount } from '../invalidate'
import { Section } from './section'

const EMPTY = { current: '', next: '', confirm: '' }

/** Why the account has no local password to change. */
function ExternalNotice({ source }: { source: MyAccount['auth_source'] }) {
  const { t } = useTranslation()
  return <p className="text-sm text-muted-foreground">{t(`me.password.external.${source}`)}</p>
}

/**
 * Password change for local accounts. The server revokes every session and hands this
 * browser a fresh one, so other devices sign in again.
 */
export function PasswordCard({ account }: { account: MyAccount }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const mutation = useChangePassword()
  const [fields, setFields] = useState(EMPTY)
  const [done, setDone] = useState(false)
  const mismatch = fields.confirm !== '' && fields.confirm !== fields.next

  const field = (name: keyof typeof EMPTY) => ({
    value: fields[name],
    onChange: (event: { target: { value: string } }) => {
      setDone(false)
      setFields((current) => ({ ...current, [name]: event.target.value }))
    },
  })

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (fields.next !== fields.confirm) return
    const data = { current_password: fields.current, new_password: fields.next }
    const me = await mutation.mutateAsync({ data }).catch(() => undefined)
    if (me === undefined) return
    setFields(EMPTY)
    setDone(true)
    await refreshAccount(queryClient, me)
  }

  return (
    <Section title={t('me.password.title')} description={t('me.password.description')}>
      {account.can_change_password ? (
        <form className="grid max-w-md gap-3" onSubmit={(event) => void submit(event)}>
          <TextField
            id="me-password-current"
            label={t('accountSetup.currentPassword')}
            type="password"
            autoComplete="current-password"
            required
            {...field('current')}
          />
          <TextField
            id="me-password-new"
            label={t('accountSetup.newPassword')}
            type="password"
            autoComplete="new-password"
            required
            {...field('next')}
          />
          <TextField
            id="me-password-confirm"
            label={t('accountSetup.confirmPassword')}
            type="password"
            autoComplete="new-password"
            required
            aria-invalid={mismatch}
            hint={mismatch ? t('setup.passwordMismatch') : undefined}
            {...field('confirm')}
          />
          {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
          <p role="status" className="text-sm text-muted-foreground">
            {done ? t('me.password.changed') : null}
          </p>
          <Button
            type="submit"
            size="sm"
            className="w-fit"
            disabled={mutation.isPending || mismatch}
          >
            {t('me.password.submit')}
          </Button>
        </form>
      ) : (
        <ExternalNotice source={account.auth_source} />
      )}
    </Section>
  )
}
