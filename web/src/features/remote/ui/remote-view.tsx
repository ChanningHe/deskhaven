import { Info, Unplug } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import type { Speaker } from '../audio/speaker'
import type { Session } from '../session/session'
import type { SessionIntent } from '../settings/protocol'
import type { RemoteSettings } from '../settings/settings'
import { PastePanel, RemoteClipboardPanel } from './clipboard-panels'
import { RemoteBar, type BarFrame } from './remote-bar'
import { RemoteScreen } from './remote-screen'
import { StatsOverlay } from './stats-overlay'
import { Toolbar } from './toolbar'
import { useClipboardSync } from './use-clipboard-sync'
import { canLockKeyboard, useKeyboardLock } from './use-keyboard-lock'
import { useStore } from './use-remote'

type Panel = 'paste' | 'remote' | null
type Notice = 'sent' | 'copiedFromDevice' | null

const NOTICE_MS = 3000

/** Reads the local clipboard; null when the browser does not let the page (no gesture, denied). */
async function readClipboard(): Promise<string | null> {
  if (typeof navigator.clipboard === 'undefined') return null
  try {
    return await navigator.clipboard.readText()
  } catch {
    return null
  }
}

// The whole document goes full screen, not the remote area: the page is already only the
// remote area, and portalled tooltips and selects stay visible this way.
function useFullscreen() {
  const [fullscreen, setFullscreen] = useState(false)
  useEffect(() => {
    const onChange = () => {
      setFullscreen(document.fullscreenElement !== null)
    }
    document.addEventListener('fullscreenchange', onChange)
    return () => {
      document.removeEventListener('fullscreenchange', onChange)
    }
  }, [])
  const toggle = () => {
    if (document.fullscreenElement) void document.exitFullscreen()
    else void document.documentElement.requestFullscreen()
  }
  return { fullscreen, toggle }
}

/** Why some controls are greyed out: the device turned them off. */
function PermissionNotice({ keyboard, clipboard }: { keyboard: boolean; clipboard: boolean }) {
  const { t } = useTranslation()
  if (keyboard && clipboard) return null
  return (
    <p className="flex shrink-0 items-start gap-2 border-b bg-muted/50 px-3 py-1.5 text-xs text-muted-foreground">
      <Info aria-hidden className="mt-px size-3.5 shrink-0" />
      <span className="flex flex-wrap gap-x-3">
        {keyboard ? null : <span>{t('remote.toolbar.keyboardOff')}</span>}
        {clipboard ? null : <span>{t('remote.toolbar.clipboardOff')}</span>}
      </span>
    </p>
  )
}

/** The active session: the bar with its controls, and the screen filling the rest. */
export function RemoteView({
  session,
  frame,
  settings,
  onSettings,
  intent,
  onIntent,
  speaker,
  onDisconnect,
}: {
  session: Session
  frame: BarFrame
  settings: RemoteSettings
  onSettings: (patch: Partial<RemoteSettings>) => void
  intent: SessionIntent
  onIntent: (patch: Partial<SessionIntent>) => void
  speaker: Speaker
  onDisconnect: () => void
}) {
  const { t } = useTranslation()
  const [notice, setNotice] = useState<Notice>(null)
  const { fullscreen, toggle } = useFullscreen()
  useKeyboardLock(fullscreen)
  const [panel, setPanel] = useState<Panel>(null)
  const [stats, setStats] = useState(false)
  const peer = useStore(session.peer)
  const remoteText = useStore(session.remoteClipboard)
  const { keyboard, clipboard } = useStore(session.permissions)
  const toolbar = useRef<HTMLDivElement>(null)
  const leaveScreen = useCallback(() => {
    toolbar.current?.querySelector<HTMLElement>('button:enabled, select:enabled')?.focus()
  }, [])
  const sync = useClipboardSync({
    session,
    enabled: settings.clipboardSync,
    viewOnly: intent.viewOnly,
    onEnabled: (clipboardSync) => {
      onSettings({ clipboardSync })
    },
    onSent: () => {
      setNotice('sent')
    },
    onCopied: () => {
      setNotice('copiedFromDevice')
    },
  })
  useEffect(() => {
    if (notice === null) return
    const timer = setTimeout(() => {
      setNotice(null)
    }, NOTICE_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [notice])
  const close = () => {
    setPanel(null)
  }
  const show = (next: Panel) => {
    setNotice(null)
    setPanel(next)
  }

  const sendClipboard = async () => {
    const text = await readClipboard()
    if (text !== null && session.clipboard(text)) {
      show(null)
      setNotice('sent')
    } else {
      show('paste')
    }
  }

  return (
    <>
      <RemoteBar
        {...frame}
        status="active"
        viewOnly={intent.viewOnly}
        hostname={peer?.info.hostname === '' ? undefined : peer?.info.hostname}
        tools={
          <Toolbar
            ref={toolbar}
            session={session}
            settings={settings}
            onSettings={onSettings}
            intent={intent}
            onIntent={onIntent}
            speaker={speaker}
            stats={stats}
            onStats={setStats}
            fullscreen={fullscreen}
            onFullscreen={toggle}
            keyboardLock={canLockKeyboard()}
            clipboardSync={{ ...sync, enabled: settings.clipboardSync }}
            onSendClipboard={() => void sendClipboard()}
            onShowRemoteClipboard={() => {
              show(panel === 'remote' ? null : 'remote')
            }}
          />
        }
        end={
          <Button variant="destructive" onClick={onDisconnect}>
            <Unplug aria-hidden />
            {t('remote.toolbar.disconnect')}
          </Button>
        }
      />
      <PermissionNotice keyboard={keyboard} clipboard={clipboard} />
      <div className="relative flex min-h-0 flex-1 flex-col bg-muted/40 dark:bg-background">
        <RemoteScreen
          session={session}
          scale={settings.scale}
          showRemoteCursor={settings.showRemoteCursor}
          viewOnly={intent.viewOnly}
          onLeave={leaveScreen}
          onPointerEnter={sync.onPointerEnter}
          onGesture={sync.onGesture}
          onPaste={sync.onPaste}
        />
        {stats ? <StatsOverlay session={session} transport={frame.transport} /> : null}
        {panel === 'paste' && clipboard ? (
          <PastePanel onSend={(text) => session.clipboard(text)} onClose={close} />
        ) : null}
        {panel === 'remote' && clipboard && remoteText !== null ? (
          <RemoteClipboardPanel text={remoteText} onClose={close} />
        ) : null}
        <p
          role="status"
          className="pointer-events-none absolute bottom-4 left-1/2 -translate-x-1/2 rounded-full bg-foreground px-3 py-1.5 text-xs whitespace-nowrap text-background shadow-md empty:hidden"
        >
          {notice === null ? null : t(`remote.clipboard.${notice}`)}
        </p>
      </div>
    </>
  )
}
