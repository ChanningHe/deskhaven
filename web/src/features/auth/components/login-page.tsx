import { useQueryClient } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Info } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { Me } from '@/api/generated/model'

import { acceptSession, useSetupStatus } from '../api'
import { landingPath } from '../session'
import { isInsecurePage } from '../transport'
import { InsecureTransportNotice } from './insecure-transport-notice'
import { AuthLayout } from './auth-layout'
import { OidcButtons } from './oidc-buttons'
import type { Challenge } from './password-step'
import { PasswordStep } from './password-step'
import { TotpStep } from './totp-step'

/** While no owner exists, points at the one-time setup link. */
function SetupHint() {
  const { t } = useTranslation()
  const status = useSetupStatus()
  if (status.data?.required !== true) return null
  return (
    <p className="flex items-start gap-2 rounded-lg bg-muted px-3 py-2 text-sm text-muted-foreground">
      <Info aria-hidden className="mt-0.5 size-4 shrink-0" />
      {t('login.setupPending')}
    </p>
  )
}

/**
 * Sign-in: password, then (with TOTP) a code step; OIDC buttons. A signed-in
 * user goes to the pending account setup step, the remembered page, or the
 * overview.
 */
export function LoginPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const { redirect } = useSearch({ from: '/login' })
  const [challenge, setChallenge] = useState<Challenge | null>(null)

  const finish = (me: Me) => {
    acceptSession(queryClient, me)
    void navigate({ href: landingPath(me, redirect), replace: true })
  }

  return (
    <AuthLayout title={challenge === null ? t('login.title') : t('login.totpTitle')}>
      {isInsecurePage(window.location) ? <InsecureTransportNotice /> : null}
      {challenge === null ? (
        <>
          <PasswordStep onSignedIn={finish} onChallenge={setChallenge} />
          <OidcButtons />
          <SetupHint />
        </>
      ) : (
        <TotpStep
          challenge={challenge}
          onSignedIn={finish}
          onRestart={() => {
            setChallenge(null)
          }}
        />
      )}
    </AuthLayout>
  )
}
