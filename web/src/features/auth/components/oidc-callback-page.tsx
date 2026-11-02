import { useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { isApiError } from '@/api/api-error'
import { buttonVariants } from '@/components/ui/button'

import { acceptSession, useCompleteOidc } from '../api'
import { LOGIN_PATH, landingPath } from '../session'
import { AuthError } from './auth-error'
import { AuthLayout } from './auth-layout'

/** The identity provider's answer, as the callback forwarded it. */
export interface OidcAnswer {
  state?: string | undefined
  code?: string | undefined
  error?: string | undefined
}

const REASONS = [
  'denied',
  'unavailable',
  'invalid_token',
  'no_linked_account',
  'disabled',
  'unverified',
  'provisioning_failed',
  'expired',
] as const

type Reason = (typeof REASONS)[number]

function reasonOf(error: unknown): Reason | undefined {
  if (!isApiError(error) || error.code !== 'oidc_failed') return undefined
  const reason = error.details?.reason
  return REASONS.find((known) => known === reason)
}

/**
 * `/login/oidc`: completes a console OIDC login in this browser (the server
 * checks the flow's binding cookie). The code and state leave the address bar
 * and history at once.
 */
export function OidcCallbackPage({ answer }: { answer: OidcAnswer }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const complete = useCompleteOidc()
  // Read once: the URL is cleaned right away, and the flow completes only once.
  const [first] = useState(answer)
  const started = useRef(false)

  useEffect(() => {
    if (started.current) return
    started.current = true
    void navigate({ to: '/login/oidc', search: {}, replace: true })
    if (first.state === undefined) return
    complete.mutate(
      { state: first.state, code: first.code ?? null, error: first.error ?? null },
      {
        onSuccess: (reply) => {
          if (reply.status !== 'signed_in') return
          acceptSession(queryClient, reply.user)
          void navigate({ href: landingPath(reply.user), replace: true })
        },
      },
    )
  }, [complete, first, navigate, queryClient])

  const reason = reasonOf(complete.error)
  const failed = first.state === undefined || complete.isError
  return (
    <AuthLayout title={failed ? t('oidc.failedTitle') : t('oidc.completing')}>
      {reason === undefined ? (
        <AuthError error={complete.error} />
      ) : (
        <p role="alert" className="text-sm text-destructive">
          {t(`oidc.reasons.${reason}`)}
        </p>
      )}
      {first.state === undefined ? (
        <p className="text-sm text-muted-foreground">{t('oidc.reasons.expired')}</p>
      ) : null}
      {failed ? (
        <Link to={LOGIN_PATH} className={buttonVariants({ variant: 'outline' })}>
          {t('oidc.backToLogin')}
        </Link>
      ) : null}
    </AuthLayout>
  )
}
