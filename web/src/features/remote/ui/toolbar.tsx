import { ClipboardCopy, ClipboardPaste, Keyboard, Maximize, Minimize, Unplug } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { NativeSelect } from '@/components/common/native-select'
import { Button } from '@/components/ui/button'

import { supportedKeyboardModes } from '../input/keyboard'
import { KeyboardMode } from '../proto/gen/message_pb.js'
import type { Session } from '../session/session'
import { useStore } from './use-remote'

const MODE_KEYS = {
  [KeyboardMode.Map]: 'map',
  [KeyboardMode.Translate]: 'translate',
  [KeyboardMode.Legacy]: 'legacy',
} as const satisfies Partial<Record<KeyboardMode, string>>

type ShownMode = keyof typeof MODE_KEYS

function isShownMode(mode: KeyboardMode): mode is ShownMode {
  return mode in MODE_KEYS
}

interface ToolbarProps {
  session: Session
  fullscreen: boolean
  onDisconnect: () => void
  onFullscreen: () => void
  onSendClipboard: () => void
  onShowRemoteClipboard: () => void
}

export function Toolbar({
  session,
  fullscreen,
  onDisconnect,
  onFullscreen,
  onSendClipboard,
  onShowRemoteClipboard,
}: ToolbarProps) {
  const { t } = useTranslation()
  const id = useId()
  const peer = useStore(session.peer)
  const { keyboard, clipboard } = useStore(session.permissions)
  const remoteText = useStore(session.remoteClipboard)
  const modes = peer
    ? supportedKeyboardModes(peer.info.platform, peer.info.version).filter(isShownMode)
    : []

  return (
    <div className="space-y-1 border-b bg-muted/50 px-3 py-2">
      <div
        role="toolbar"
        aria-label={t('remote.toolbar.label')}
        className="flex flex-wrap items-center gap-2"
      >
        <Button variant="destructive" size="sm" onClick={onDisconnect}>
          <Unplug aria-hidden />
          {t('remote.toolbar.disconnect')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={!keyboard}
          onClick={() => {
            session.ctrlAltDel()
          }}
        >
          {t('remote.toolbar.ctrlAltDel')}
        </Button>
        <span className="inline-flex items-center gap-1.5">
          <Keyboard aria-hidden className="size-4 text-muted-foreground" />
          <label htmlFor={`${id}-mode`} className="sr-only">
            {t('remote.toolbar.keyboardMode')}
          </label>
          <NativeSelect
            id={`${id}-mode`}
            className="h-7"
            disabled={!keyboard || modes.length < 2}
            value={peer?.keyboardMode ?? ''}
            onChange={(event) => {
              const mode = modes.find((m) => String(m) === event.target.value)
              if (mode !== undefined) session.setKeyboardMode(mode)
            }}
          >
            {modes.map((mode) => (
              <option key={mode} value={mode}>
                {t(`remote.toolbar.modes.${MODE_KEYS[mode]}`)}
              </option>
            ))}
          </NativeSelect>
        </span>
        {peer && peer.displays.length > 1 ? (
          <span className="inline-flex items-center gap-1.5">
            <label htmlFor={`${id}-display`} className="text-sm text-muted-foreground">
              {t('remote.toolbar.display')}
            </label>
            <NativeSelect
              id={`${id}-display`}
              className="h-7"
              value={peer.current}
              onChange={(event) => {
                session.switchDisplay(Number(event.target.value))
              }}
            >
              {peer.displays.map((_, index) => (
                <option key={index} value={index}>
                  {t('remote.toolbar.displayN', { n: index + 1 })}
                </option>
              ))}
            </NativeSelect>
          </span>
        ) : null}
        <Button variant="outline" size="sm" disabled={!clipboard} onClick={onSendClipboard}>
          <ClipboardPaste aria-hidden />
          {t('remote.toolbar.sendClipboard')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={!clipboard || remoteText === null}
          onClick={onShowRemoteClipboard}
        >
          <ClipboardCopy aria-hidden />
          {t('remote.toolbar.remoteClipboard')}
        </Button>
        <Button variant="outline" size="sm" onClick={onFullscreen}>
          {fullscreen ? <Minimize aria-hidden /> : <Maximize aria-hidden />}
          {t(fullscreen ? 'remote.toolbar.exitFullscreen' : 'remote.toolbar.fullscreen')}
        </Button>
      </div>
      {keyboard ? null : (
        <p className="text-xs text-muted-foreground">{t('remote.toolbar.keyboardOff')}</p>
      )}
      {clipboard ? null : (
        <p className="text-xs text-muted-foreground">{t('remote.toolbar.clipboardOff')}</p>
      )}
    </div>
  )
}
