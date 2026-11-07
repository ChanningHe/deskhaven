import { useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { isApiError } from '@/api/api-error'
import { useCompleteOidcLink, useVerifyOidcLink } from '@/api/generated/me/me'
import type { OidcLinkPreview } from '@/api/generated/model'
import { PageHeader } from '@/components/common/page-header'
import { Button, buttonVariants } from '@/components/ui/button'
import type { OidcAnswer } from '@/features/auth/components/oidc-callback-page'

import { refreshAccount } from '../invalidate'
import { LinkFailure } from './link-failure'

/** Asking again while another request verifies the same answer (409 `verifying`). */
const RETRY_DELAY_MS = 1000
const MAX_RETRIES = 10

function isVerifying(error: unknown): boolean {
  return isApiError(error) && error.code === 'conflict' && error.details?.reason === 'verifying'
}

function BackLink() {
  const { t } = useTranslation()
  return (
    <Link to="/me" className={buttonVariants({ variant: 'outline' })}>
      {t('me.identities.back')}
    </Link>
  )
}

/** The IdP account about to be linked, as the provider asserted it. */
function Shown({ shown }: { shown: OidcLinkPreview }) {
  const { t } = useTranslation()
  const rows = [
    [t('me.identities.fields.subject'), shown.subject],
    [t('me.identities.fields.email'), shown.email],
    [t('me.identities.fields.username'), shown.preferred_username],
  ] as const
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
      {rows.map(([label, value]) =>
        value === null || value === undefined ? null : (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="break-all">{value}</dd>
          </div>
        ),
      )}
    </dl>
  )
}

function Confirmation({ state, shown }: { state: string; shown: OidcLinkPreview }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const complete = useCompleteOidcLink()
  // The flow is single use: one request, whatever the clicks before the next render.
  const sent = useRef(false)
  const busy = complete.isPending
  const confirm = () => {
    if (sent.current) return
    sent.current = true
    complete.mutate(
      { data: { state } },
      {
        onSuccess: () => {
          void refreshAccount(queryClient).then(() => navigate({ to: '/me' }))
        },
      },
    )
  }
  return (
    <div className="space-y-4">
      <p className="text-sm">
        {t('me.identities.confirmQuestion', { provider: shown.provider, username: shown.username })}
      </p>
      <Shown shown={shown} />
      <p role="note" className="text-sm text-warning">
        {t('me.identities.warning')}
      </p>
      {complete.isError ? <LinkFailure error={complete.error} /> : null}
      <div className="flex gap-2">
        <Button
          onClick={confirm}
          disabled={busy || complete.isSuccess || complete.isError}
          aria-busy={busy}
        >
          {t('me.identities.confirmSubmit')}
        </Button>
        <Link to="/me" className={buttonVariants({ variant: 'outline' })}>
          {t('common.cancel')}
        </Link>
      </div>
    </div>
  )
}

/**
 * `/me/oidc-link`: the identity provider's answer to a link started on the account page.
 * The server exchanges it once and names the IdP account and this account; confirming
 * links exactly that IdP account (only in this browser and for this account). The code
 * and state leave the address bar at once; an IdP refusal is shown without a request.
 */
export function OidcLinkPage({ answer }: { answer: OidcAnswer }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  // Read once: the URL is cleaned right away, and the answer is verified only once.
  const [first] = useState(answer)
  const started = useRef(false)
  const verify = useVerifyOidcLink()
  const state = first.state ?? ''
  const refusedAtIdp = first.error !== undefined

  const [retries, setRetries] = useState(0)
  const busyElsewhere = isVerifying(verify.error) && retries < MAX_RETRIES

  useEffect(() => {
    if (started.current) return
    started.current = true
    void navigate({ to: '/me/oidc-link', search: {}, replace: true })
    if (state === '' || refusedAtIdp) return
    verify.mutate({ data: { state, code: first.code ?? null, error: null } })
  }, [first, navigate, refusedAtIdp, state, verify])

  // Another request (another tab) is exchanging this answer: ask again shortly; the
  // server answers from the flow once it is done, without a second exchange.
  useEffect(() => {
    if (!busyElsewhere) return
    const timer = setTimeout(() => {
      setRetries((count) => count + 1)
      verify.mutate({ data: { state, code: first.code ?? null, error: null } })
    }, RETRY_DELAY_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [busyElsewhere, first, state, verify])

  let failure
  if (state === '') failure = <LinkFailure reason="expired" />
  else if (refusedAtIdp) failure = <LinkFailure reason="denied" />
  else if (verify.isError && !busyElsewhere) failure = <LinkFailure error={verify.error} />
  let body
  if (failure !== undefined) {
    body = (
      <>
        {failure}
        <BackLink />
      </>
    )
  } else if (verify.data === undefined) {
    body = (
      <p role="status" className="text-sm text-muted-foreground">
        {t('me.identities.verifying')}
      </p>
    )
  } else {
    body = <Confirmation state={state} shown={verify.data} />
  }
  return (
    <div className="max-w-xl space-y-6">
      <PageHeader title={t('me.identities.confirmTitle')} />
      {body}
    </div>
  )
}
