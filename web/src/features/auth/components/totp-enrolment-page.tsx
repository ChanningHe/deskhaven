import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { beginTotpEnrolment, confirmTotpEnrolment } from '@/api/generated/me/me'
import type { TotpConfirm } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Button } from '@/components/ui/button'

import { acceptSession, useLogout } from '../api'
import { landingPath } from '../session'
import { AuthError } from './auth-error'
import { AuthLayout } from './auth-layout'
import { QrCode } from './qr-code'
import { TextField } from './text-field'

/** `ABCDEFGH…` as `ABCD EFGH …`, easier to type into an authenticator. */
function grouped(secret: string): string {
  return secret.replace(/(.{4})(?=.)/g, '$1 ')
}

/**
 * `/account/two-factor`: `auth.tfa.enforce` requires TOTP and the account has
 * none. Scan (or type) the new seed, confirm a code, save the recovery codes;
 * the answer carries a fresh session.
 */
export function TotpEnrolmentPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  // gcTime 0: the seed is not kept in the cache once the page is gone.
  const begin = useMutation({ mutationFn: () => beginTotpEnrolment(), gcTime: 0 })
  const confirm = useMutation({
    mutationFn: (body: TotpConfirm) => confirmTotpEnrolment(body),
    gcTime: 0,
  })
  const logout = useLogout()
  const [code, setCode] = useState('')
  const started = useRef(false)

  useEffect(() => {
    if (started.current) return
    started.current = true
    begin.mutate()
  }, [begin])

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    confirm.mutate(
      { code: code.trim() },
      {
        onError: () => {
          setCode('')
        },
      },
    )
  }

  const enabled = confirm.data
  const done = () => {
    if (enabled === undefined) return
    const me = enabled.user
    confirm.reset()
    begin.reset()
    acceptSession(queryClient, me)
    void navigate({ href: landingPath(me), replace: true })
  }

  const enrolment = begin.data
  return (
    <AuthLayout
      title={t('accountSetup.totpTitle')}
      description={t('accountSetup.totpDescription')}
      wide
    >
      {begin.isError ? (
        <ErrorAlert
          error={begin.error}
          onRetry={() => {
            begin.mutate()
          }}
        />
      ) : null}
      {enrolment === undefined ? null : (
        <div className="space-y-4">
          <div className="flex flex-col items-center gap-3">
            <QrCode qr={enrolment.qr} label={t('accountSetup.qrLabel')} />
            <p className="text-sm text-muted-foreground">{t('accountSetup.manual')}</p>
            <code className="rounded bg-muted px-2 py-1 font-mono text-sm break-all">
              {grouped(enrolment.secret)}
            </code>
          </div>
          <form method="post" className="space-y-4" onSubmit={submit}>
            <TextField
              id="totp-code"
              label={t('accountSetup.code')}
              autoComplete="one-time-code"
              inputMode="numeric"
              required
              value={code}
              onChange={(event) => {
                setCode(event.target.value)
              }}
            />
            <AuthError error={confirm.error} />
            <Button type="submit" className="w-full" disabled={confirm.isPending}>
              {t('accountSetup.enable')}
            </Button>
          </form>
        </div>
      )}
      <Button
        variant="ghost"
        className="w-full"
        onClick={() => {
          logout.mutate()
        }}
      >
        {t('user.signOut')}
      </Button>
      <SecretRevealDialog
        open={enabled !== undefined}
        title={t('accountSetup.recoveryTitle')}
        description={t('accountSetup.recoveryDescription')}
        secret={enabled?.recovery_codes.join('\n') ?? ''}
        onDone={done}
      />
    </AuthLayout>
  )
}
