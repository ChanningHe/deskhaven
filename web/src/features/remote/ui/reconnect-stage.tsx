import { Unplug } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'
import { FrozenScreen } from './frozen-screen'
import { RemoteBar, type BarFrame } from './remote-bar'
import type { Frozen } from './use-screen'

export function DisconnectButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation()
  return (
    <Button variant="destructive" onClick={onClick}>
      <Unplug aria-hidden />
      {t('remote.toolbar.disconnect')}
    </Button>
  )
}

/** The page reconnecting after the link dropped: the bar, and the session's last frame. */
export function ReconnectStage({
  frame,
  frozen,
  attempt,
  settings,
  intent,
  onDisconnect,
}: {
  frame: BarFrame
  frozen: Frozen
  attempt: number
  settings: RemoteSettings
  intent: SessionIntent
  onDisconnect: () => void
}) {
  const { t } = useTranslation()
  return (
    <>
      <RemoteBar
        {...frame}
        status="connecting"
        viewOnly={intent.viewOnly}
        hostname={frozen.hostname}
        end={<DisconnectButton onClick={onDisconnect} />}
      />
      <div className="relative flex min-h-0 flex-1 flex-col bg-muted/40 dark:bg-background">
        <FrozenScreen
          canvas={frozen.canvas}
          scale={settings.scale}
          hint={t('remote.resumingAttempt', { attempt })}
        />
      </div>
    </>
  )
}
