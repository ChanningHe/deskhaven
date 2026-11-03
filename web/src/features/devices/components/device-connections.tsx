import { useState } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import { disconnectDevice } from '@/api/generated/devices/devices'
import type { ConnectionView, DeviceDetail } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { Button } from '@/components/ui/button'
import { formatDateTime } from '@/lib/format'

/** Connection types the console names (`conn_type` of the audit message). */
const KNOWN_TYPES = [0, 1, 2, 3, 4] as const

/** Live connections with a way to close each one, and the recent connection history. */
export function DeviceConnections({ detail }: { detail: DeviceDetail }) {
  return (
    <>
      <LiveConnections deviceId={detail.device.id} connections={detail.connections} />
      <RecentConnections connections={detail.recent_connections} />
    </>
  )
}

function LiveConnections({ deviceId, connections }: { deviceId: number; connections: number[] }) {
  const { t } = useTranslation()
  // `target` outlives `open`: the dialog keeps its texts while it animates closed.
  const [target, setTarget] = useState(0)
  const [confirming, setConfirming] = useState(false)
  const [ttl, setTtl] = useState<number | undefined>(undefined)
  const [queued, setQueued] = useState(false)
  return (
    <section aria-label={t('devices.connections.title')} className="grid gap-2">
      <h3 className="text-sm font-medium">{t('devices.connections.title')}</h3>
      {connections.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('devices.connections.none')}</p>
      ) : (
        <ul className="grid gap-1 text-sm">
          {connections.map((id) => (
            <li key={id} className="flex items-center justify-between gap-2">
              <span>{t('devices.connections.connection', { id })}</span>
              <Button
                variant="outline"
                size="sm"
                aria-label={t('devices.connections.disconnectNamed', { id })}
                onClick={() => {
                  setTarget(id)
                  setConfirming(true)
                }}
              >
                {t('devices.connections.disconnect')}
              </Button>
            </li>
          ))}
        </ul>
      )}
      <p role="status" className="text-xs text-muted-foreground">
        {queued ? t('devices.connections.queued') : null}
      </p>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('devices.connections.confirmTitle')}
        description={t('devices.connections.confirmDescription', {
          id: target,
          seconds: ttl ?? DEFAULT_TTL_SECONDS,
        })}
        confirmLabel={t('devices.connections.confirmAction')}
        onConfirm={async () => {
          const result = await disconnectDevice(deviceId, { conn_id: target })
          setTtl(result.ttl_seconds)
          setQueued(true)
        }}
      />
    </section>
  )
}

/** Shown before the server has said how long a request lives (`DISCONNECT_TTL`). */
const DEFAULT_TTL_SECONDS = 60

function RecentConnections({ connections }: { connections: ConnectionView[] }) {
  const { t, i18n } = useTranslation()
  return (
    <section aria-label={t('devices.connections.recent')} className="grid gap-2">
      <h3 className="text-sm font-medium">{t('devices.connections.recent')}</h3>
      {connections.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('devices.connections.noRecent')}</p>
      ) : (
        <ul className="grid gap-2 text-sm">
          {connections.map((connection) => (
            <li key={connection.guid} className="grid gap-0.5">
              <span className="flex justify-between gap-2">
                <span>{typeName(connection.conn_type, t)}</span>
                <span className="text-xs text-muted-foreground">
                  {connection.ended_at === null || connection.ended_at === undefined
                    ? t('devices.connections.active')
                    : t('devices.connections.ended')}
                </span>
              </span>
              <span className="text-xs text-muted-foreground">
                {t('devices.connections.from', {
                  peer:
                    connection.peer_name ??
                    connection.peer_rustdesk_id ??
                    t('devices.connections.unknownPeer'),
                })}
                {connection.ip ? ` · ${connection.ip}` : ''}
                {connection.started_at === null || connection.started_at === undefined
                  ? ''
                  : ` · ${formatDateTime(connection.started_at, i18n.language)}`}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function typeName(type: number | null | undefined, t: TFunction) {
  if (type === null || type === undefined) return ''
  const known = KNOWN_TYPES.find((candidate) => candidate === type)
  return known === undefined
    ? t('devices.connections.otherType', { type })
    : t(`devices.connections.types.${known}`)
}
