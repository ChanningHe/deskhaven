import { useTranslation } from 'react-i18next'

import type { DeviceView } from '@/api/generated/model'
import { cn } from '@/lib/utils'

type Shown = 'online' | 'offline' | 'disabled'

const DOT_CLASS: Record<Shown, string> = {
  online: 'bg-emerald-600 dark:bg-emerald-400',
  offline: 'bg-muted-foreground/40',
  disabled: 'bg-destructive',
}

function shownOf(device: Pick<DeviceView, 'online' | 'status'>): Shown {
  if (device.status === 'disabled') return 'disabled'
  return device.online ? 'online' : 'offline'
}

/** A status dot with its word (never colour alone): online, offline or disabled. */
export function DeviceStatusBadge({ device }: { device: Pick<DeviceView, 'online' | 'status'> }) {
  const { t } = useTranslation()
  const shown = shownOf(device)
  return (
    <span className="inline-flex items-center gap-1.5 text-sm">
      <span aria-hidden className={cn('size-2 rounded-full', DOT_CLASS[shown])} />
      {t(`devices.status.${shown}`)}
    </span>
  )
}
