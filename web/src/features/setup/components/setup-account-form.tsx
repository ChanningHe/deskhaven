import { useMutation } from '@tanstack/react-query'
import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { completeSetup } from '@/api/generated/setup/setup'
import type { Me, SetupRequest } from '@/api/generated/model'
import { Button } from '@/components/ui/button'
import { AuthError } from '@/features/auth/components/auth-error'
import { TextField } from '@/features/auth/components/text-field'

interface SetupAccountFormProps {
  token: string
  /** The owner exists and is signed in. */
  onCreated: (me: Me) => void
}

const EMPTY = { username: '', displayName: '', email: '', password: '', confirm: '' }

/** Step 1: the owner account, created with the setup link's token. */
export function SetupAccountForm({ token, onCreated }: SetupAccountFormProps) {
  const { t } = useTranslation()
  const create = useMutation({ mutationFn: (body: SetupRequest) => completeSetup(body) })
  const [fields, setFields] = useState(EMPTY)
  const mismatch = fields.confirm !== '' && fields.confirm !== fields.password

  const field = (name: keyof typeof EMPTY) => ({
    value: fields[name],
    onChange: (event: { target: { value: string } }) => {
      setFields((current) => ({ ...current, [name]: event.target.value }))
    },
  })

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (fields.password !== fields.confirm) return
    const request = {
      token,
      username: fields.username,
      display_name: fields.displayName === '' ? null : fields.displayName,
      email: fields.email === '' ? null : fields.email,
      password: fields.password,
    }
    create.mutate(request, {
      onSuccess: (reply) => {
        if (reply.status === 'signed_in') onCreated(reply.user)
      },
    })
  }

  return (
    <form method="post" className="space-y-4" onSubmit={submit}>
      <TextField
        id="setup-username"
        label={t('setup.username')}
        autoComplete="username"
        required
        {...field('username')}
      />
      <TextField
        id="setup-display-name"
        label={t('setup.displayName')}
        hint={t('setup.optional')}
        autoComplete="name"
        {...field('displayName')}
      />
      <TextField
        id="setup-email"
        label={t('setup.email')}
        hint={t('setup.optional')}
        type="email"
        autoComplete="email"
        {...field('email')}
      />
      <TextField
        id="setup-password"
        label={t('setup.password')}
        type="password"
        autoComplete="new-password"
        required
        {...field('password')}
      />
      <TextField
        id="setup-confirm"
        label={t('setup.confirmPassword')}
        type="password"
        autoComplete="new-password"
        required
        aria-invalid={mismatch}
        hint={mismatch ? t('setup.passwordMismatch') : undefined}
        {...field('confirm')}
      />
      <AuthError error={create.error} />
      <Button type="submit" className="w-full" disabled={create.isPending || mismatch}>
        {t('setup.create')}
      </Button>
    </form>
  )
}
