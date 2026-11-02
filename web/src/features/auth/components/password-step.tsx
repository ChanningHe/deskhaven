import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import type { Me } from '@/api/generated/model'
import { Button } from '@/components/ui/button'

import { useLogin } from '../api'
import { AuthError } from './auth-error'
import { TextField } from './text-field'

/** The password step answered with a TOTP challenge. */
export interface Challenge {
  username: string
  challenge: string
}

interface PasswordStepProps {
  onSignedIn: (me: Me) => void
  onChallenge: (challenge: Challenge) => void
}

export function PasswordStep({ onSignedIn, onChallenge }: PasswordStepProps) {
  const { t } = useTranslation()
  const login = useLogin()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    // Never let the browser submit (a GET would leak the password into the URL).
    event.preventDefault()
    login.mutate(
      { username, password },
      {
        onSuccess: (reply) => {
          setPassword('')
          if (reply.status === 'signed_in') onSignedIn(reply.user)
          else onChallenge({ username, challenge: reply.challenge })
        },
      },
    )
  }

  return (
    <form method="post" className="space-y-4" onSubmit={submit}>
      <TextField
        id="login-username"
        label={t('login.username')}
        name="username"
        autoComplete="username"
        required
        value={username}
        onChange={(event) => {
          setUsername(event.target.value)
        }}
      />
      <TextField
        id="login-password"
        label={t('login.password')}
        name="password"
        type="password"
        autoComplete="current-password"
        required
        value={password}
        onChange={(event) => {
          setPassword(event.target.value)
        }}
      />
      <AuthError error={login.error} />
      <Button type="submit" className="w-full" disabled={login.isPending}>
        {login.isPending ? t('login.signingIn') : t('login.submit')}
      </Button>
    </form>
  )
}
