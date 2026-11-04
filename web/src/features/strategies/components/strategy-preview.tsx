import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { usePreviewStrategy } from '@/api/generated/strategies/strategies'
import { ErrorAlert } from '@/components/common/error-alert'
import { DeviceStrategy } from '@/features/devices/components/device-strategy'

import { DevicePicker } from './device-picker'

/**
 * What a device ends up with: every strategy that applies to it merged, each option with
 * its source, as its next heartbeat delivers it (saved changes only).
 */
export function StrategyPreview() {
  const { t } = useTranslation()
  const [device, setDevice] = useState<number | undefined>(undefined)
  const query = usePreviewStrategy(
    { device_id: device ?? 0 },
    { query: { enabled: device !== undefined } },
  )
  return (
    <section className="grid gap-4">
      <p className="text-xs text-muted-foreground">{t('strategies.preview.description')}</p>
      <DevicePicker value={device} onChange={setDevice} />
      {query.error ? <ErrorAlert error={query.error} onRetry={() => void query.refetch()} /> : null}
      {device !== undefined && query.data !== undefined ? (
        <DeviceStrategy strategy={query.data} />
      ) : null}
    </section>
  )
}
