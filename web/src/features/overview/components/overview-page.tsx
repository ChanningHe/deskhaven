import { useTranslation } from 'react-i18next'

import type { OverviewView } from '@/api/generated/model'
import { useGetOverview } from '@/api/generated/overview/overview'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'

import { RecentAlarms } from './recent-alarms'
import { StatTile } from './stat-tile'
import { VersionChart } from './version-chart'

/** The numbers follow the fleet within seconds; TanStack Query pauses while the tab is hidden. */
const REFRESH_MS = 10_000

export function OverviewPage() {
  const { t } = useTranslation()
  const query = useGetOverview({ query: { refetchInterval: REFRESH_MS } })
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.overview')} description={t('pages.overview.description')} />
      {query.data === undefined ? (
        query.error ? (
          <ErrorAlert
            error={query.error}
            title={t('overview.loadFailed')}
            onRetry={() => void query.refetch()}
          />
        ) : (
          <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
        )
      ) : (
        <>
          {/* A failed refresh keeps the last numbers and says so. */}
          {query.error ? (
            <ErrorAlert error={query.error} title={t('overview.refreshFailed')} />
          ) : null}
          <Overview data={query.data} />
        </>
      )}
    </div>
  )
}

function Overview({ data }: { data: OverviewView }) {
  const { t, i18n } = useTranslation()
  const format = (n: number) => new Intl.NumberFormat(i18n.language).format(n)
  const { online, total } = data.devices
  return (
    <div className="space-y-8">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatTile
          label={t('overview.tiles.devices')}
          value={`${format(online)} / ${format(total)}`}
          hint={t('overview.tiles.devicesHint')}
          meter={{ value: online, max: total, label: t('overview.tiles.devicesMeter') }}
        />
        <StatTile
          label={t('overview.tiles.activeConnections')}
          value={format(data.active_connections)}
          hint={t('overview.tiles.activeConnectionsHint')}
        />
        <StatTile
          label={t('overview.tiles.connections24h')}
          value={format(data.connections_24h)}
          hint={t('overview.tiles.window')}
        />
        <StatTile
          label={t('overview.tiles.failedLogins')}
          value={format(data.failed_logins_24h)}
          hint={t('overview.tiles.window')}
          attention={data.failed_logins_24h > 0}
        />
      </div>
      <div className="grid gap-8 lg:grid-cols-2">
        <VersionChart versions={data.client_versions} format={format} />
        <RecentAlarms alarms={data.recent_alarms} />
      </div>
    </div>
  )
}
