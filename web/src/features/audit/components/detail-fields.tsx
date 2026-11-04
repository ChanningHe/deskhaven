import { useId } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { formatDateTime, nonBlank } from '@/lib/format'

/** A `name: value` grid. Values are rendered as text nodes only (06 §2). */
export function Fields({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">{children}</dl>
}

export function Field({ name, children }: { name: ReactNode; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{name}</dt>
      <dd className="min-w-0 break-words whitespace-pre-wrap">
        <bdi>{children}</bdi>
      </dd>
    </>
  )
}

/** `value`, or a dash for a missing or empty one. */
export function Optional({ value }: { value: string | null | undefined }) {
  const { t } = useTranslation()
  const shown = nonBlank(value)
  if (shown === undefined)
    return <span className="text-muted-foreground">{t('audit.notReported')}</span>
  return <bdi>{shown}</bdi>
}

export function Time({ at }: { at: number | null | undefined }) {
  const { t, i18n } = useTranslation()
  if (at === null || at === undefined) {
    return <span className="text-muted-foreground">{t('audit.notReported')}</span>
  }
  return <>{formatDateTime(at, i18n.language)}</>
}

/**
 * What the device (or client) reported about itself, labelled as such: the server cannot
 * verify it (06 §2).
 */
export function SelfReported({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="grid gap-2">
      <h3 id={headingId} className="text-sm font-medium">
        {t('audit.detail.selfReported')}
      </h3>
      <p className="text-xs text-muted-foreground">{t('audit.detail.selfReportedHint')}</p>
      {children}
    </section>
  )
}
