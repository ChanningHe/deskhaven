import { useEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'
import { useTranslation } from 'react-i18next'

import type { Session } from '../session/session'
import { PastePanel, RemoteClipboardPanel } from './clipboard-panels'
import { RemoteScreen } from './remote-screen'
import { Toolbar } from './toolbar'
import { useStore } from './use-remote'

type Panel = 'paste' | 'remote' | null

/** Reads the local clipboard; null when the browser does not let the page (no gesture, denied). */
async function readClipboard(): Promise<string | null> {
  if (typeof navigator.clipboard === 'undefined') return null
  try {
    return await navigator.clipboard.readText()
  } catch {
    return null
  }
}

function useFullscreen(element: RefObject<HTMLElement | null>) {
  const [fullscreen, setFullscreen] = useState(false)
  useEffect(() => {
    const onChange = () => {
      setFullscreen(
        document.fullscreenElement !== null && document.fullscreenElement === element.current,
      )
    }
    document.addEventListener('fullscreenchange', onChange)
    return () => {
      document.removeEventListener('fullscreenchange', onChange)
    }
  }, [element])
  const toggle = () => {
    if (document.fullscreenElement) void document.exitFullscreen()
    else void element.current?.requestFullscreen()
  }
  return { fullscreen, toggle }
}

/** The active session: toolbar, clipboard strips and the screen, together in full screen. */
export function RemoteView({
  session,
  onDisconnect,
}: {
  session: Session
  onDisconnect: () => void
}) {
  const { t } = useTranslation()
  const area = useRef<HTMLDivElement>(null)
  const [sent, setSent] = useState(false)
  const { fullscreen, toggle } = useFullscreen(area)
  const [panel, setPanel] = useState<Panel>(null)
  const remoteText = useStore(session.remoteClipboard)
  const { clipboard } = useStore(session.permissions)
  const close = () => {
    setPanel(null)
  }
  const show = (next: Panel) => {
    setSent(false)
    setPanel(next)
  }

  const sendClipboard = async () => {
    const text = await readClipboard()
    if (text !== null && session.clipboard(text)) {
      show(null)
      setSent(true)
    } else {
      show('paste')
    }
  }

  return (
    <div
      ref={area}
      className="flex h-[calc(100svh-10rem)] min-h-80 flex-col overflow-hidden rounded-xl border bg-card [&:fullscreen]:h-svh [&:fullscreen]:rounded-none [&:fullscreen]:border-0"
    >
      <Toolbar
        session={session}
        fullscreen={fullscreen}
        onDisconnect={onDisconnect}
        onFullscreen={toggle}
        onSendClipboard={() => void sendClipboard()}
        onShowRemoteClipboard={() => {
          show(panel === 'remote' ? null : 'remote')
        }}
      />
      <p role="status" className="px-3 text-xs text-muted-foreground empty:hidden">
        {sent ? t('remote.clipboard.sent') : null}
      </p>
      {panel === 'paste' && clipboard ? (
        <PastePanel onSend={(text) => session.clipboard(text)} onClose={close} />
      ) : null}
      {panel === 'remote' && clipboard && remoteText !== null ? (
        <RemoteClipboardPanel text={remoteText} onClose={close} />
      ) : null}
      <RemoteScreen session={session} />
    </div>
  )
}
