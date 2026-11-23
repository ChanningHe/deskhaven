import { Maximize, Minimize } from 'lucide-react'
import type { Ref } from 'react'
import { useTranslation } from 'react-i18next'

import type { Speaker } from '../audio/speaker'
import type { Session } from '../session/session'
import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'
import { ActionsMenu } from './menus/actions-menu'
import { ClipboardMenu, type ClipboardSyncSwitch } from './menus/clipboard-menu'
import { DisplayMenu } from './menus/display-menu'
import { InputMenu } from './menus/input-menu'
import { SoundButton } from './sound-button'
import { IconButton } from './toolbar-controls'

interface ToolbarProps {
  ref?: Ref<HTMLDivElement>
  session: Session
  settings: RemoteSettings
  onSettings: (patch: Partial<RemoteSettings>) => void
  intent: SessionIntent
  onIntent: (patch: Partial<SessionIntent>) => void
  speaker: Speaker
  stats: boolean
  onStats: (on: boolean) => void
  fullscreen: boolean
  onFullscreen: () => void
  keyboardLock: boolean
  clipboardSync: ClipboardSyncSwitch
  onSendClipboard: () => void
  onShowRemoteClipboard: () => void
}

export function Toolbar({
  ref,
  session,
  settings,
  onSettings,
  intent,
  onIntent,
  speaker,
  stats,
  onStats,
  fullscreen,
  onFullscreen,
  keyboardLock,
  clipboardSync,
  onSendClipboard,
  onShowRemoteClipboard,
}: ToolbarProps) {
  const { t } = useTranslation()
  const fullscreenLabel = t(
    fullscreen ? 'remote.toolbar.exitFullscreen' : 'remote.toolbar.fullscreen',
  )
  return (
    <div
      ref={ref}
      role="toolbar"
      aria-label={t('remote.toolbar.label')}
      className="flex min-w-0 flex-wrap items-stretch gap-0.5 max-lg:gap-1"
    >
      <DisplayMenu
        session={session}
        viewOnly={intent.viewOnly}
        settings={settings}
        onSettings={onSettings}
        stats={stats}
        onStats={onStats}
      />
      <InputMenu session={session} viewOnly={intent.viewOnly} />
      <ClipboardMenu
        session={session}
        viewOnly={intent.viewOnly}
        sync={clipboardSync}
        onSend={onSendClipboard}
        onShowRemote={onShowRemoteClipboard}
      />
      <ActionsMenu session={session} intent={intent} onIntent={onIntent} />
      <span aria-hidden className="mx-1 my-2 w-px bg-border" />
      <SoundButton
        session={session}
        speaker={speaker}
        enabled={settings.audio}
        onEnabled={(audio) => {
          onSettings({ audio })
        }}
      />
      <IconButton
        label={fullscreenLabel}
        hint={
          keyboardLock
            ? fullscreenLabel
            : t('remote.toolbar.noKeyboardLock', { label: fullscreenLabel })
        }
        onClick={onFullscreen}
      >
        {fullscreen ? <Minimize aria-hidden /> : <Maximize aria-hidden />}
      </IconButton>
    </div>
  )
}
