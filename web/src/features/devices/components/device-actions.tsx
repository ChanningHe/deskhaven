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
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'

import { invalidateDevices } from '../invalidate'

/** Actions that ask for confirmation first (08 §1.4). */
type Confirmed = 'disable' | 'delete'

interface DeviceActionsProps {
  device: DeviceView
  onDeleted: () => void
}

/** Disable / enable and delete, in the drawer. */
export function DeviceActions({ device, onDeleted }: DeviceActionsProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  // `action` outlives `open`: the dialog keeps its texts while it animates closed.
  const [action, setAction] = useState<Confirmed>('disable')
  const [confirming, setConfirming] = useState(false)
  const update = useUpdateDevice()
  const remove = useDeleteDevice()
  const refresh = () => invalidateDevices(queryClient)

  const confirmed: Record<Confirmed, () => Promise<unknown>> = {
    disable: async () =>
      update.mutateAsync({ id: device.id, data: { status: 'disabled' } }).then(refresh),
    delete: async () => {
      await remove.mutateAsync({ id: device.id })
      onDeleted()
      // Gone: nothing to refetch (it would only 404).
      queryClient.removeQueries({ queryKey: getGetDeviceQueryKey(device.id) })
      await refresh()
    },
  }

  const ask = (next: Confirmed) => () => {
    setAction(next)
    setConfirming(true)
  }

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
          <Button variant="outline" size="sm" onClick={ask('disable')}>
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
        <Button variant="destructive" size="sm" onClick={ask('delete')}>
          {t('devices.actions.delete')}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t('devices.actions.disableHint')}</p>
      {update.error ? <ErrorAlert error={update.error} /> : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t(`devices.confirm.${action}.title`)}
        description={t(`devices.confirm.${action}.description`, { id: device.rustdesk_id })}
        confirmLabel={t(`devices.confirm.${action}.confirm`)}
        {...(action === 'delete' ? { confirmText: device.rustdesk_id } : {})}
        onConfirm={confirmed[action]}
      />
    </section>
  )
}
