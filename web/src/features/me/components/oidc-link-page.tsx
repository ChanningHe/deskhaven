import { useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { isApiError } from '@/api/api-error'
import { useCompleteOidcLink, usePreviewOidcLink } from '@/api/generated/me/me'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'
import { Button, buttonVariants } from '@/components/ui/button'
import type { OidcAnswer } from '@/features/auth/components/oidc-callback-page'

import { refreshAccount } from '../invalidate'

const LINK_REASONS = [
  'denied',
  'unavailable',
  'invalid_token',
  'identity_taken',
  'provider_linked',
  'directory_account',
  'account_inactive',
  'expired',
] as const

type Reason = (typeof LINK_REASONS)[number]

function reasonOf(error: unknown): Reason | undefined {
  if (!isApiError(error) || error.code !== 'oidc_failed') return undefined
  const reason = error.details?.reason
  return LINK_REASONS.find((known) => known === reason)
}

function Failure({ error }: { error: unknown }) {
  const { t } = useTranslation()
  const reason = reasonOf(error)
  if (reason === undefined) return <ErrorAlert error={error} />
  return (
    <p role="alert" className="text-sm text-destructive">
      {t(`me.identities.reasons.${reason}`)}
    </p>
  )
}

/**
 * `/me/oidc-link`: the identity provider's answer to a link started on the account page.
 * Names the provider and the account, and links on confirmation (the server checks the
 * flow's binding cookie and that it was started for this account). The code and state
 * leave the address bar at once.
 */
export function OidcLinkPage({ answer }: { answer: OidcAnswer }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  // Read once: the URL is cleaned right away.
  const [first] = useState(answer)
  const cleaned = useRef(false)
  const state = first.state ?? ''
  const preview = usePreviewOidcLink({ state }, { query: { enabled: state !== '', retry: false } })
  const complete = useCompleteOidcLink()

  useEffect(() => {
    if (cleaned.current) return
    cleaned.current = true
    void navigate({ to: '/me/oidc-link', search: {}, replace: true })
  }, [navigate])

  const confirm = () => {
    const data = { state, code: first.code ?? null, error: first.error ?? null }
    complete.mutate(
      { data },
      {
        onSuccess: () => {
          void refreshAccount(queryClient).then(() => navigate({ to: '/me' }))
        },
      },
    )
  }

  const shown = preview.data
  return (
    <div className="max-w-xl space-y-6">
      <PageHeader title={t('me.identities.confirmTitle')} />
      {state === '' ? (
        <p className="text-sm text-muted-foreground">{t('me.identities.reasons.expired')}</p>
      ) : null}
      {preview.isError ? <Failure error={preview.error} /> : null}
      {shown === undefined ? null : (
        <div className="space-y-4">
          <p className="text-sm">
            {t('me.identities.confirmQuestion', {
              provider: shown.provider,
              username: shown.username,
            })}
          </p>
          <p role="note" className="text-sm text-amber-700 dark:text-amber-400">
            {t('me.identities.warning')}
          </p>
          {complete.isError ? <Failure error={complete.error} /> : null}
          <div className="flex gap-2">
            <Button onClick={confirm} disabled={complete.isPending || complete.isError}>
              {t('me.identities.confirmSubmit')}
            </Button>
            <Link to="/me" className={buttonVariants({ variant: 'outline' })}>
              {t('common.cancel')}
            </Link>
          </div>
        </div>
      )}
      {shown === undefined ? (
        <Link to="/me" className={buttonVariants({ variant: 'outline' })}>
          {t('me.identities.back')}
        </Link>
      ) : null}
    </div>
  )
}
