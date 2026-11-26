import { Link } from '@tanstack/react-router'
import { ArrowRight, BellRing } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import type { AlarmAuditView } from '@/api/generated/model'
import { EmptyState } from '@/components/common/empty-state'
import { SectionCard } from '@/components/common/section-card'
import { buttonVariants } from '@/components/ui/button'
import { alarmTypeLabel } from '@/features/audit/audit-labels'
import { formatDateTime } from '@/lib/format'

export function RecentAlarms({ alarms }: { alarms: AlarmAuditView[] }) {
  const { t, i18n } = useTranslation()
  return (
    <SectionCard
      title={t('overview.alarms.title')}
      description={t('overview.alarms.selfReported')}
      actions={
        <Link
          to="/audit"
          search={{ tab: 'alarms' }}
          className={buttonVariants({ variant: 'outline', size: 'sm' })}
        >
          {t('overview.alarms.viewAll')}
          <ArrowRight data-icon="inline-end" aria-hidden />
        </Link>
      }
    >
      {alarms.length === 0 ? (
        <EmptyState icon={BellRing} headingLevel={3} title={t('overview.alarms.empty')} />
      ) : (
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
      )}
    </SectionCard>
  )
}
