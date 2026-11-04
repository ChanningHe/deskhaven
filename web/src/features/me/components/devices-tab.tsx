import { MonitorSmartphone } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { useListMyDevices } from '@/api/generated/me/me'
import type { MyDevice } from '@/api/generated/model'
import { EmptyState } from '@/components/common/empty-state'
import { ErrorAlert } from '@/components/common/error-alert'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { DeviceStatusBadge } from '@/features/devices/components/device-status-badge'
import { formatDateTime, nonBlank } from '@/lib/format'

import { Section } from './section'

/** Everyone sees their devices in one page: assigned devices are few. */
const PAGE_SIZE = 200

/** Self-reported text, isolated from the surrounding text; a dash when missing. */
function Reported({ value }: { value: string | null | undefined }) {
  const text = nonBlank(value)
  return text === undefined ? <span className="text-muted-foreground">—</span> : <bdi>{text}</bdi>
}

function DeviceRow({ device, language }: { device: MyDevice; language: string }) {
  return (
    <TableRow>
      <TableCell className="font-mono text-xs">{device.rustdesk_id}</TableCell>
      <TableCell>
        <Reported value={device.hostname} />
      </TableCell>
      <TableCell>
        <Reported value={device.os} />
      </TableCell>
      <TableCell>
        <Reported value={device.version} />
      </TableCell>
      <TableCell>
        <DeviceStatusBadge device={device} />
      </TableCell>
      <TableCell>{formatDateTime(device.last_seen_at, language)}</TableCell>
    </TableRow>
  )
}

/** The devices the user owns, read only, with their online state. */
export function DevicesTab() {
  const { t, i18n } = useTranslation()
  const query = useListMyDevices({ page_size: PAGE_SIZE })
  const devices = query.data?.items ?? []
  const total = query.data?.total ?? 0

  return (
    <Section title={t('me.devices.title')} description={t('me.devices.description')}>
      {query.isError ? (
        <ErrorAlert
          error={query.error}
          title={t('me.devices.loadFailed')}
          onRetry={() => void query.refetch()}
        />
      ) : null}
      {query.isSuccess && devices.length === 0 ? (
        <EmptyState
          icon={MonitorSmartphone}
          title={t('me.devices.emptyTitle')}
          description={t('me.devices.emptyDescription')}
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t('me.devices.columns.id')}</TableHead>
              <TableHead>{t('me.devices.columns.hostname')}</TableHead>
              <TableHead>{t('me.devices.columns.os')}</TableHead>
              <TableHead>{t('me.devices.columns.version')}</TableHead>
              <TableHead>{t('me.devices.columns.status')}</TableHead>
              <TableHead>{t('me.devices.columns.lastSeen')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {devices.map((device) => (
              <DeviceRow key={device.rustdesk_id} device={device} language={i18n.language} />
            ))}
          </TableBody>
        </Table>
      )}
      {total > devices.length ? (
        <p className="text-xs text-muted-foreground">
          {t('me.devices.truncated', { shown: devices.length, total })}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">{t('me.devices.selfReported')}</p>
    </Section>
  )
}
