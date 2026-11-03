import { useTranslation } from 'react-i18next'

import { useGetDevice } from '@/api/generated/devices/devices'
import { nonBlank } from '@/lib/format'
import { httpStatusOf } from '@/lib/http-error'
import type { DeviceDetail } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'

import { DeviceActions } from './device-actions'
import { DeviceConnections } from './device-connections'
import { DeviceManageForm } from './device-manage-form'
import { DeviceReported } from './device-reported'
import { DeviceStatusBadge } from './device-status-badge'
import { DeviceStrategy } from './device-strategy'

const HTTP_NOT_FOUND = 404

/** The open drawer follows the device: its connections change within seconds. */
const DETAIL_REFRESH_MS = 10_000

interface DeviceDrawerProps {
  /** The device to show; undefined closes the drawer. */
  deviceId: number | undefined
  onClose: () => void
}

/** Side drawer of one device: ownership, self-reported data, connections, strategy, actions. */
export function DeviceDrawer({ deviceId, onClose }: DeviceDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={deviceId !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-md">
        {deviceId === undefined ? null : <DrawerBody deviceId={deviceId} onDeleted={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({ deviceId, onDeleted }: { deviceId: number; onDeleted: () => void }) {
  const { t } = useTranslation()
  const query = useGetDevice(deviceId, { query: { refetchInterval: DETAIL_REFRESH_MS } })
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('devices.drawer.title')}</SheetTitle>
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <SheetDescription>{t('common.loading')}</SheetDescription>
        )}
      </SheetHeader>
    )
  }
  // A later poll failed: the data below is the last good copy.
  const failure = query.isError ? query.error : undefined
  return (
    <Detail
      detail={query.data}
      failure={failure}
      onRetry={() => void query.refetch()}
      onDeleted={onDeleted}
    />
  )
}

interface DetailProps {
  detail: DeviceDetail
  failure: unknown
  onRetry: () => void
  onDeleted: () => void
}

function Detail({ detail, failure, onRetry, onDeleted }: DetailProps) {
  const { t } = useTranslation()
  const { device } = detail
  const gone = httpStatusOf(failure) === HTTP_NOT_FOUND
  return (
    <>
      <SheetHeader>
        <SheetTitle>{nonBlank(device.hostname) ?? device.rustdesk_id}</SheetTitle>
        <SheetDescription>
          {t('devices.drawer.description', { id: device.rustdesk_id })}
        </SheetDescription>
        <div className="pt-2">
          <DeviceStatusBadge device={device} />
        </div>
      </SheetHeader>
      {gone ? (
        <p role="alert" className="mx-4 rounded-lg bg-muted px-3 py-2 text-sm">
          {t('devices.drawer.gone')}
        </p>
      ) : (
        <LiveDetail
          detail={detail}
          stale={failure === undefined ? undefined : { error: failure, onRetry }}
          onDeleted={onDeleted}
        />
      )}
    </>
  )
}

function LiveDetail({
  detail,
  stale,
  onDeleted,
}: {
  detail: DeviceDetail
  stale: { error: unknown; onRetry: () => void } | undefined
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const { device } = detail
  return (
    <div className="grid gap-6 px-4 pb-6">
      {stale === undefined ? null : (
        <ErrorAlert error={stale.error} title={t('devices.drawer.stale')} onRetry={stale.onRetry} />
      )}
      <DeviceManageForm key={device.id} device={device} />
      <DeviceReported detail={detail} />
      <DeviceConnections detail={detail} />
      <DeviceStrategy strategy={detail.strategy} />
      <DeviceActions device={device} onDeleted={onDeleted} />
    </div>
  )
}
