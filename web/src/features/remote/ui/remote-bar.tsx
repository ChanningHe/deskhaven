import { cn } from '@/lib/utils'
import { useEffect } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

export type BarStatus = 'connecting' | 'active' | 'ended'
export type Transport = 'relay' | 'webrtc'

const DOT: Record<BarStatus, string> = {
  connecting: 'bg-warning motion-safe:animate-pulse',
  active: 'bg-success',
  ended: 'bg-muted-foreground/60',
}

/** What every state of the remote page shares: the way back, the device, its link state. */
export interface BarFrame {
  deviceId: string
  /** The way back to the console; the route supplies it so the page needs no router. */
  back?: ReactNode
  /** The link the session runs over, once the race has a winner. */
  transport?: Transport | undefined
}

interface RemoteBarProps extends BarFrame {
  status: BarStatus
  /** The peer's own name, once it has said it. */
  hostname?: string | undefined
  /** The session's controls; they drop to a second row on narrow screens. */
  tools?: ReactNode
  /** The last control, kept on the first row: the session's way out. */
  end?: ReactNode
}

/** The thin bar over the remote area: the page's only chrome. */
export function RemoteBar({
  deviceId,
  back,
  status,
  hostname,
  transport,
  tools,
  end,
}: RemoteBarProps) {
  const { t } = useTranslation()
  const title = hostname ?? t('remote.device', { id: deviceId })

  useEffect(() => {
    document.title = `${title} · ${t('remote.title')} · ${t('app.name')}`
  }, [title, t])

  return (
    <header className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b bg-background px-2 py-1.5">
      {back}
      <div className="flex min-w-0 flex-1 items-center gap-2.5 pl-1">
        <h1 className="truncate text-sm font-semibold">{title}</h1>
        {hostname === undefined ? null : (
          <span className="hidden font-mono text-xs text-muted-foreground tabular-nums sm:inline">
            {deviceId}
          </span>
        )}
        <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
          <span aria-hidden className={cn('size-1.5 rounded-full', DOT[status])} />
          {t(`remote.status.${status}`)}
          {transport === undefined ? null : (
            <>
              <span aria-hidden className="text-border">
                |
              </span>
              {t(`remote.transport.${transport}`)}
            </>
          )}
        </span>
      </div>
      {tools ? (
        <div className="order-last flex w-full min-w-0 lg:order-none lg:w-auto">{tools}</div>
      ) : null}
      {end}
    </header>
  )
}
