import { Link } from '@tanstack/react-router'
import { BellRing } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import type { AlarmAuditView } from '@/api/generated/model'
import { alarmTypeLabel } from '@/features/audit/audit-labels'
import { formatDateTime } from '@/lib/format'

/** The latest alarms; the device ids and details are what the devices reported. */
export function RecentAlarms({ alarms }: { alarms: AlarmAuditView[] }) {
  const { t, i18n } = useTranslation()
  return (
    <section aria-labelledby="overview-alarms" className="space-y-3">
      <div className="flex items-baseline justify-between gap-2">
        <h2 id="overview-alarms" className="text-sm font-medium">
          {t('overview.alarms.title')}
        </h2>
        <Link
          to="/audit"
          search={{ tab: 'alarms' }}
          className="text-xs text-muted-foreground underline-offset-4 hover:underline"
        >
          {t('overview.alarms.viewAll')}
        </Link>
      </div>
      {alarms.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('overview.alarms.empty')}</p>
      ) : (
        <>
          <p className="text-xs text-muted-foreground">{t('overview.alarms.selfReported')}</p>
          <ul className="divide-y rounded-lg border">
            {alarms.map((alarm) => (
              <li key={alarm.id} className="flex items-start gap-3 px-3 py-2 text-sm">
                <BellRing aria-hidden className="mt-0.5 size-4 shrink-0 text-warning" />
                <div className="min-w-0 flex-1">
                  <p className="font-medium">{alarmTypeLabel(t, alarm.typ)}</p>
                  <p className="truncate font-mono text-xs text-muted-foreground">
                    <bdi>{alarm.device_id}</bdi>
                    {alarm.info.length === 0 ? null : (
                      <>
                        {' · '}
                        <bdi>{alarm.info.map((e) => `${e.key}=${e.value}`).join(' ')}</bdi>
                      </>
                    )}
                  </p>
                </div>
                <time
                  dateTime={new Date(alarm.created_at).toISOString()}
                  className="shrink-0 text-xs text-muted-foreground"
                >
                  {formatDateTime(alarm.created_at, i18n.language)}
                </time>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  )
}
