import { useQueryClient } from '@tanstack/react-query'
import { Link, useLocation, useNavigate } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { Me } from '@/api/generated/model'
import { Button, buttonVariants } from '@/components/ui/button'
import { acceptSession, useSetupStatus } from '@/features/auth/api'
import { AuthLayout } from '@/features/auth/components/auth-layout'
import { InsecureTransportNotice } from '@/features/auth/components/insecure-transport-notice'
import { LOGIN_PATH, landingPath } from '@/features/auth/session'
import { isInsecurePage } from '@/features/auth/transport'
import { cn } from '@/lib/utils'

import { SetupAccountForm } from './setup-account-form'

const STEPS = ['account', 'publicUrl', 'done'] as const
type Step = (typeof STEPS)[number]

function Steps({ current }: { current: Step }) {
  const { t } = useTranslation()
  return (
    <ol className="flex gap-2 text-xs">
      {STEPS.map((step, index) => (
        <li
          key={step}
          aria-current={step === current ? 'step' : undefined}
          className={cn(
            'flex-1 rounded-md border px-2 py-1 text-center',
            step === current ? 'border-primary text-foreground' : 'text-muted-foreground',
          )}
        >
          {index + 1}. {t(`setup.steps.${step}`)}
        </li>
      ))}
    </ol>
  )
}

/** The `token` of `#token=…`; the fragment never reaches a server or a log. */
function tokenOf(hash: string): string | undefined {
  const token = new URLSearchParams(hash.replace(/^#/, '')).get('token')
  return token === null || token === '' ? undefined : token
}

function PublicUrlStep({ publicUrl, onNext }: { publicUrl: string | null; onNext: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="space-y-4 text-sm">
      <p className="text-muted-foreground">{t('setup.publicUrlHint')}</p>
      <p className="rounded-lg bg-muted px-3 py-2 font-mono break-all">
        {publicUrl ?? t('setup.publicUrlUnset')}
      </p>
      <Button className="w-full" onClick={onNext}>
        {t('setup.next')}
      </Button>
    </div>
  )
}

/**
 * `/setup#token=…`: first-run setup in three steps (08 §3.2): create the owner
 * (who is then signed in), review the public URL, finish.
 */
export function SetupPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const location = useLocation()
  const status = useSetupStatus()
  const [token] = useState(() => tokenOf(location.hash))
  const [step, setStep] = useState<Step>('account')
  const [owner, setOwner] = useState<Me | null>(null)

  useEffect(() => {
    // Drop the token from the address bar and history.
    if (location.hash !== '') void navigate({ to: '/setup', replace: true })
  }, [location.hash, navigate])

  const created = (me: Me) => {
    acceptSession(queryClient, me)
    setOwner(me)
    setStep('publicUrl')
  }

  const closed = step === 'account' && status.data?.required === false
  return (
    <AuthLayout title={t('setup.title')} description={t('setup.description')} wide>
      <Steps current={step} />
      {isInsecurePage(window.location) ? <InsecureTransportNotice /> : null}
      {closed ? (
        <div className="space-y-4 text-sm">
          <p>{t('setup.completed')}</p>
          <Link to={LOGIN_PATH} className={buttonVariants({ variant: 'outline' })}>
            {t('login.submit')}
          </Link>
        </div>
      ) : null}
      {!closed && step === 'account' && token === undefined ? (
        <p role="alert" className="text-sm text-muted-foreground">
          {t('setup.missingToken')}
        </p>
      ) : null}
      {!closed && step === 'account' && token !== undefined ? (
        <SetupAccountForm token={token} onCreated={created} />
      ) : null}
      {step === 'publicUrl' ? (
        <PublicUrlStep
          publicUrl={status.data?.public_url ?? null}
          onNext={() => {
            setStep('done')
          }}
        />
      ) : null}
      {step === 'done' && owner !== null ? (
        <div className="space-y-4 text-sm">
          <p className="text-muted-foreground">{t('setup.doneDescription')}</p>
          <Button
            className="w-full"
            onClick={() => void navigate({ href: landingPath(owner), replace: true })}
          >
            {t('setup.openConsole')}
          </Button>
        </div>
      ) : null}
    </AuthLayout>
  )
}
