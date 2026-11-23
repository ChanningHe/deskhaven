import { Maximize, Minimize } from 'lucide-react'
import type { Ref } from 'react'
import { useTranslation } from 'react-i18next'

import type { Session } from '../session/session'
import type { RemoteSettings } from '../settings/settings'
import { ClipboardMenu, type ClipboardSyncSwitch } from './menus/clipboard-menu'
import { DisplayMenu } from './menus/display-menu'
import { InputMenu } from './menus/input-menu'
import { IconButton } from './toolbar-controls'

interface ToolbarProps {
  ref?: Ref<HTMLDivElement>
  session: Session
  settings: RemoteSettings
  onSettings: (patch: Partial<RemoteSettings>) => void
  stats: boolean
  onStats: (on: boolean) => void
  fullscreen: boolean
  onFullscreen: () => void
  clipboardSync: ClipboardSyncSwitch
  onSendClipboard: () => void
  onShowRemoteClipboard: () => void
}

export function Toolbar({
  ref,
  session,
  settings,
  onSettings,
  stats,
  onStats,
  fullscreen,
  onFullscreen,
  clipboardSync,
  onSendClipboard,
  onShowRemoteClipboard,
}: ToolbarProps) {
  const { t } = useTranslation()
  return (
    <div
      ref={ref}
      role="toolbar"
      aria-label={t('remote.toolbar.label')}
      className="flex min-w-0 flex-wrap items-stretch gap-0.5"
    >
      <DisplayMenu
        session={session}
        settings={settings}
        onSettings={onSettings}
        stats={stats}
        onStats={onStats}
      />
      <InputMenu session={session} />
      <ClipboardMenu
        session={session}
        sync={clipboardSync}
        onSend={onSendClipboard}
        onShowRemote={onShowRemoteClipboard}
      />
      <IconButton
        label={t(fullscreen ? 'remote.toolbar.exitFullscreen' : 'remote.toolbar.fullscreen')}
        onClick={onFullscreen}
      >
        {fullscreen ? <Minimize aria-hidden /> : <Maximize aria-hidden />}
      </IconButton>
    </div>
  )
}
