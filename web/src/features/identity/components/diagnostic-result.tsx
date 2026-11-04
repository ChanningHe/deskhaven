import { CircleCheck, CircleMinus, CircleX } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import type { LdapDiagnosticView, LdapProbedUserView, LdapStepStatus } from '@/api/generated/model'
import { cn } from '@/lib/utils'

const STATUS_STYLE = {
  passed: { icon: CircleCheck, className: 'text-info' },
  failed: { icon: CircleX, className: 'text-destructive' },
  skipped: { icon: CircleMinus, className: 'text-muted-foreground' },
} as const satisfies Record<LdapStepStatus, unknown>

/** The steps of an LDAP test, each with its status and message, then the user found. */
export function DiagnosticResult({ result }: { result: LdapDiagnosticView }) {
  const { t } = useTranslation()
  return (
    <section aria-label={t('identity.diagnostics.resultLabel')} className="grid gap-3">
      <p
        role="status"
        className={cn('text-sm font-medium', result.passed ? 'text-info' : 'text-destructive')}
      >
        {result.passed ? t('identity.diagnostics.passed') : t('identity.diagnostics.failed')}
      </p>
      <ol className="grid gap-2">
        {result.steps.map((step) => {
          const style = STATUS_STYLE[step.status]
          const Icon = style.icon
          return (
            <li key={step.step} data-step={step.step} className="flex items-start gap-2 text-sm">
              <Icon aria-hidden className={cn('mt-0.5 size-4 shrink-0', style.className)} />
              <div className="min-w-0">
                <p>
                  <span className="font-medium">
                    {t(`identity.diagnostics.steps.${step.step}`)}
                  </span>
                  {' · '}
                  <span className={style.className}>
                    {t(`identity.diagnostics.status.${step.status}`)}
                  </span>
                </p>
                <p className="text-xs break-all text-muted-foreground">{step.message}</p>
              </div>
            </li>
          )
        })}
      </ol>
      {result.user == null ? null : <ProbedUser user={result.user} />}
    </section>
  )
}

function ProbedUser({ user }: { user: LdapProbedUserView }) {
  const { t } = useTranslation()
  const list = (values: readonly string[]) =>
    values.length === 0 ? t('identity.diagnostics.user.none') : values.join(', ')
  const admin =
    user.admin == null
      ? t('identity.diagnostics.user.noAdminGroup')
      : user.admin
        ? t('identity.diagnostics.user.yes')
        : t('identity.diagnostics.user.no')
  const rows: readonly (readonly [string, string])[] = [
    [t('identity.diagnostics.user.dn'), user.dn],
    [t('identity.diagnostics.user.username'), user.username ?? '—'],
    [t('identity.diagnostics.user.email'), user.email ?? '—'],
    [t('identity.diagnostics.user.displayName'), user.display_name ?? '—'],
    [t('identity.diagnostics.user.directoryGroups'), list(user.directory_groups)],
    [t('identity.diagnostics.user.mappedGroups'), list(user.mapped_groups)],
    [t('identity.diagnostics.user.admin'), admin],
  ]
  return (
    <div className="rounded-lg border p-3">
      <h4 className="mb-2 text-sm font-medium">{t('identity.diagnostics.user.title')}</h4>
      <dl className="grid gap-1 text-xs sm:grid-cols-[max-content_1fr] sm:gap-x-4">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="break-all">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  )
}
