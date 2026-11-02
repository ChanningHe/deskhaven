import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { changePassword } from '@/api/generated/me/me'
import type { PasswordChange } from '@/api/generated/model'
import { Button } from '@/components/ui/button'

import { acceptSession, useLogout } from '../api'
import { landingPath } from '../session'
import { AuthError } from './auth-error'
import { AuthLayout } from './auth-layout'
import { TextField } from './text-field'

const EMPTY = { current: '', next: '', confirm: '' }

/**
 * `/account/password`: a temporary password must be replaced before anything
 * else (the server restricts the session until then). The answer carries a
 * fresh session.
 */
export function ChangePasswordPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const change = useMutation({ mutationFn: (body: PasswordChange) => changePassword(body) })
  const logout = useLogout()
  const [fields, setFields] = useState(EMPTY)
  const mismatch = fields.confirm !== '' && fields.confirm !== fields.next

  const field = (name: keyof typeof EMPTY) => ({
    value: fields[name],
    onChange: (event: { target: { value: string } }) => {
      setFields((current) => ({ ...current, [name]: event.target.value }))
    },
  })

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (fields.next !== fields.confirm) return
    change.mutate(
      { current_password: fields.current, new_password: fields.next },
      {
        onSuccess: (me) => {
          setFields(EMPTY)
          acceptSession(queryClient, me)
          void navigate({ href: landingPath(me), replace: true })
        },
      },
    )
  }

  return (
    <AuthLayout
      title={t('accountSetup.passwordTitle')}
      description={t('accountSetup.passwordDescription')}
    >
      <form method="post" className="space-y-4" onSubmit={submit}>
        <TextField
          id="password-current"
          label={t('accountSetup.currentPassword')}
          type="password"
          autoComplete="current-password"
          required
          {...field('current')}
        />
        <TextField
          id="password-new"
          label={t('accountSetup.newPassword')}
          type="password"
          autoComplete="new-password"
          required
          {...field('next')}
        />
        <TextField
          id="password-confirm"
          label={t('accountSetup.confirmPassword')}
          type="password"
          autoComplete="new-password"
          required
          aria-invalid={mismatch}
          hint={mismatch ? t('setup.passwordMismatch') : undefined}
          {...field('confirm')}
        />
        <AuthError error={change.error} />
        <Button type="submit" className="w-full" disabled={change.isPending || mismatch}>
          {t('accountSetup.change')}
        </Button>
      </form>
      <Button
        variant="ghost"
        className="w-full"
        onClick={() => {
          logout.mutate()
        }}
      >
        {t('user.signOut')}
      </Button>
    </AuthLayout>
  )
}
