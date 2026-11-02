import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { isApiError } from '@/api/api-error'
import type { Me } from '@/api/generated/model'
import { Button } from '@/components/ui/button'

import { useTotpLogin } from '../api'
import { AuthError } from './auth-error'
import type { Challenge } from './password-step'
import { TextField } from './text-field'

interface TotpStepProps {
  challenge: Challenge
  onSignedIn: (me: Me) => void
  /** Back to the password step (the challenge expired, or the user wants to). */
  onRestart: () => void
}

/** The second factor: a TOTP code, or one of the recovery codes. */
export function TotpStep({ challenge, onSignedIn, onRestart }: TotpStepProps) {
  const { t } = useTranslation()
  const verify = useTotpLogin()
  const [code, setCode] = useState('')
  const expired = isApiError(verify.error) && verify.error.code === 'invalid_challenge'

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    verify.mutate(
      { ...challenge, code: code.trim() },
      {
        onSuccess: (reply) => {
          if (reply.status === 'signed_in') onSignedIn(reply.user)
        },
        onError: () => {
          setCode('')
        },
      },
    )
  }

  return (
    <form method="post" className="space-y-4" onSubmit={submit}>
      <p className="text-sm text-muted-foreground">{t('login.totpDescription')}</p>
      <TextField
        id="login-code"
        label={t('login.code')}
        hint={t('login.codeHint')}
        name="code"
        autoComplete="one-time-code"
        required
        autoFocus
        value={code}
        onChange={(event) => {
          setCode(event.target.value)
        }}
      />
      <AuthError error={verify.error} />
      <div className="flex gap-2">
        <Button type="button" variant="outline" onClick={onRestart}>
          {t('login.back')}
        </Button>
        <Button type="submit" className="flex-1" disabled={verify.isPending || expired}>
          {t('login.verify')}
        </Button>
      </div>
    </form>
  )
}
