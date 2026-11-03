import { useId } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { DeviceDetail } from '@/api/generated/model'
import { formatDateTime } from '@/lib/format'

const REPORTED = ['hostname', 'username', 'os', 'cpu', 'memory', 'version'] as const

/**
 * What the device says about itself, labelled as such (06 §2), and below it what the
 * server itself observed (addresses and times).
 */
export function DeviceReported({ detail }: { detail: DeviceDetail }) {
  const { t, i18n } = useTranslation()
  const headingId = useId()
  const { device } = detail
  const time = (ms: number) => formatDateTime(ms, i18n.language)
  return (
    <>
      <section aria-labelledby={headingId} className="grid gap-2">
        <h3 id={headingId} className="text-sm font-medium">
          {t('devices.drawer.selfReported')}
        </h3>
        <p className="text-xs text-muted-foreground">{t('devices.drawer.selfReportedHint')}</p>
        <Fields>
          {REPORTED.map((key) => (
            <Field key={key} name={t(`devices.drawer.fields.${key}`)}>
              {device[key] ?? (
                <span className="text-muted-foreground">{t('devices.drawer.notReported')}</span>
              )}
            </Field>
          ))}
        </Fields>
      </section>
      <Fields>
        <Field name={t('devices.drawer.fields.lastIp')}>{device.last_ip}</Field>
        <Field name={t('devices.drawer.fields.registerIp')}>{detail.register_ip}</Field>
        <Field name={t('devices.drawer.fields.firstSeen')}>{time(device.first_seen_at)}</Field>
        <Field name={t('devices.drawer.fields.lastSeen')}>{time(device.last_seen_at)}</Field>
      </Fields>
    </>
  )
}

function Fields({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">{children}</dl>
}

function Field({ name, children }: { name: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{name}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </>
  )
}
