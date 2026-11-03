import { useTranslation } from 'react-i18next'

import { useGetDevice } from '@/api/generated/devices/devices'
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
  return <Detail detail={query.data} onDeleted={onDeleted} />
}

function Detail({ detail, onDeleted }: { detail: DeviceDetail; onDeleted: () => void }) {
  const { t } = useTranslation()
  const { device } = detail
  return (
    <>
      <SheetHeader>
        <SheetTitle>{device.hostname ?? device.rustdesk_id}</SheetTitle>
        <SheetDescription>
          {t('devices.drawer.description', { id: device.rustdesk_id })}
        </SheetDescription>
        <div className="pt-2">
          <DeviceStatusBadge device={device} />
        </div>
      </SheetHeader>
      <div className="grid gap-6 px-4 pb-6">
        <DeviceManageForm key={device.id} device={device} />
        <DeviceReported detail={detail} />
        <DeviceConnections detail={detail} />
        <DeviceStrategy strategy={detail.strategy} />
        <DeviceActions device={device} onDeleted={onDeleted} />
      </div>
    </>
  )
}
