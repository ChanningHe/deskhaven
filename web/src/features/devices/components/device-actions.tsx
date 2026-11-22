import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  getGetDeviceQueryKey,
  useDeleteDevice,
  useUpdateDevice,
} from '@/api/generated/devices/devices'
import type { DeviceView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { DangerZone } from '@/components/common/danger-zone'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'

import { invalidateDevices } from '../invalidate'

/** Disable / enable in the drawer; disabling asks for confirmation first (08 §1.4). */
export function DeviceActions({ device }: { device: DeviceView }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const update = useUpdateDevice()
  const refresh = () => invalidateDevices(queryClient)

  /** Enabling needs no confirmation; the button stays disabled until it settles. */
  const enable = () => {
    if (update.isPending) return
    update.mutate(
      { id: device.id, data: { status: 'active' } },
      { onSuccess: () => void refresh() },
    )
  }

  return (
    <section aria-label={t('devices.actions.title')} className="grid gap-2">
      <h3 className="text-sm font-medium">{t('devices.actions.title')}</h3>
      <div className="flex flex-wrap gap-2">
        {device.status === 'active' ? (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setConfirming(true)
            }}
          >
            {t('devices.actions.disable')}
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            disabled={update.isPending}
            aria-busy={update.isPending}
            onClick={enable}
          >
            {t('devices.actions.enable')}
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">{t('devices.actions.disableHint')}</p>
      {update.error ? <ErrorAlert error={update.error} /> : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('devices.confirm.disable.title')}
        description={t('devices.confirm.disable.description', { id: device.rustdesk_id })}
        confirmLabel={t('devices.confirm.disable.confirm')}
        onConfirm={async () =>
          update.mutateAsync({ id: device.id, data: { status: 'disabled' } }).then(refresh)
        }
      />
    </section>
  )
}

/** Deleting the device, at the bottom of the drawer; asks for its RustDesk ID first. */
export function DeviceDangerZone({
  device,
  onDeleted,
}: {
  device: DeviceView
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const remove = useDeleteDevice()
  return (
    <DangerZone
      label={t('devices.actions.delete')}
      title={t('devices.confirm.delete.title')}
      description={t('devices.confirm.delete.description', { id: device.rustdesk_id })}
      confirmLabel={t('devices.confirm.delete.confirm')}
      confirmText={device.rustdesk_id}
      onConfirm={async () => {
        await remove.mutateAsync({ id: device.id })
        onDeleted()
        // Gone: nothing to refetch (it would only 404).
        queryClient.removeQueries({ queryKey: getGetDeviceQueryKey(device.id) })
        await invalidateDevices(queryClient)
      }}
    />
  )
}
