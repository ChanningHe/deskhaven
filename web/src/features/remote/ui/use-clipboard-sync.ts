import { useEffect, useEffectEvent, useRef, useState } from 'react'

import { ClipboardSync, syncAllowed } from '../clipboard/sync'
import type { Session } from '../session/session'
import { useStore } from './use-remote'

export type ClipboardAccess = PermissionState | 'unsupported'

export interface ClipboardSyncControl {
  access: ClipboardAccess | null
  setEnabled: (on: boolean) => void
  requestAccess: () => void
  onPointerEnter: () => void
  /** Writing needs a gesture: call inside a pointerdown or keydown handler. */
  onGesture: () => void
  onPaste: (text: string) => void
  /** Sends a text the user picked, sync on or off; false when the session refuses it. */
  send: (text: string) => boolean
}

const POLL_MS = 1000

const syncs = new WeakMap<Session, ClipboardSync>()

function syncOf(session: Session): ClipboardSync {
  let sync = syncs.get(session)
  if (!sync) {
    sync = new ClipboardSync()
    syncs.set(session, sync)
  }
  return sync
}

function clipboardApi(): Clipboard | null {
  return typeof navigator.clipboard === 'undefined' ? null : navigator.clipboard
}

function queryReadAccess(): Promise<PermissionStatus> | null {
  if (typeof navigator.permissions === 'undefined') return null
  try {
    return navigator.permissions.query({ name: 'clipboard-read' as PermissionName })
  } catch {
    return null
  }
}

function write(clipboard: Clipboard, sync: ClipboardSync, text: string): Promise<boolean> {
  sync.writeStarts()
  return clipboard.writeText(text).then(
    () => {
      sync.wrote(text)
      return true
    },
    () => false,
  )
}

function useReadAccess(): ClipboardAccess | null {
  const [query] = useState(queryReadAccess)
  const [access, setAccess] = useState<ClipboardAccess | null>(query ? null : 'unsupported')
  useEffect(() => {
    if (!query) return
    let status: PermissionStatus | null = null
    let live = true
    const update = () => {
      if (status) setAccess(status.state)
    }
    query.then(
      (answer) => {
        if (!live) return
        status = answer
        update()
        answer.addEventListener('change', update)
      },
      () => {
        if (live) setAccess('unsupported')
      },
    )
    return () => {
      live = false
      status?.removeEventListener('change', update)
    }
  }, [query])
  return access
}

export function useClipboardSync({
  session,
  enabled,
  viewOnly = false,
  onEnabled,
  onSent,
  onCopied,
}: {
  session: Session
  enabled: boolean
  viewOnly?: boolean
  onEnabled: (on: boolean) => void
  onSent: () => void
  onCopied: () => void
}): ClipboardSyncControl {
  const sync = syncOf(session)
  const access = useReadAccess()
  const active = useStore(session.state).kind === 'active'
  const { clipboard: permitted } = useStore(session.permissions)
  const allowed = syncAllowed({ active, clipboard: permitted, clipboardSync: enabled, viewOnly })
  const reading = allowed && access === 'granted'
  const readNow = useRef<(() => void) | null>(null)
  const sent = useEffectEvent(onSent)
  const copied = useEffectEvent(onCopied)

  useEffect(() => {
    const clipboard = clipboardApi()
    if (!reading || !clipboard) return
    let live = true
    let busy = false
    let timer: ReturnType<typeof setInterval> | undefined
    const read = () => {
      if (busy || !document.hasFocus()) return
      busy = true
      const started = sync.readStarts()
      clipboard
        .readText()
        .then(
          (text) => {
            const out = live ? sync.local(text, started) : null
            if (out !== null && session.clipboard(out)) sent()
          },
          () => undefined,
        )
        .finally(() => {
          busy = false
        })
    }
    const stopPolling = () => {
      clearInterval(timer)
      timer = undefined
    }
    const startPolling = () => {
      read()
      timer ??= setInterval(read, POLL_MS)
    }
    const onVisibility = () => {
      if (document.visibilityState === 'visible') read()
    }
    sync.baseline(null)
    readNow.current = read
    if (document.hasFocus()) startPolling()
    window.addEventListener('focus', startPolling)
    window.addEventListener('blur', stopPolling)
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      live = false
      readNow.current = null
      stopPolling()
      window.removeEventListener('focus', startPolling)
      window.removeEventListener('blur', stopPolling)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [reading, session, sync])

  useEffect(() => {
    const clipboard = clipboardApi()
    if (!allowed || !clipboard) return
    const onRemote = () => {
      const text = session.remoteClipboard.get()
      if (text === null) return
      const pending = sync.remote(text)
      if (pending === null) return
      if (!document.hasFocus()) {
        sync.dropPending()
        return
      }
      void write(clipboard, sync, pending).then((done) => {
        if (done) copied()
      })
    }
    const drop = () => {
      sync.dropPending()
    }
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') drop()
    }
    const unsubscribe = session.remoteClipboard.subscribe(onRemote)
    window.addEventListener('blur', drop)
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      unsubscribe()
      drop()
      window.removeEventListener('blur', drop)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [allowed, session, sync])

  const requestAccess = () => {
    const clipboard = clipboardApi()
    if (!clipboard || access === 'unsupported') return
    clipboard.readText().then(
      (text) => {
        sync.baseline(text)
      },
      () => undefined,
    )
  }

  return {
    access,
    setEnabled: (on) => {
      if (on && !enabled) requestAccess()
      onEnabled(on)
    },
    requestAccess,
    onPointerEnter: () => {
      readNow.current?.()
    },
    onPaste: (text) => {
      if (!allowed) return
      const out = sync.pasted(text)
      if (out !== null && session.clipboard(out)) onSent()
    },
    send: (text) => {
      const sent = session.clipboard(text)
      if (sent) sync.pasted(text)
      return sent
    },
    onGesture: () => {
      const clipboard = clipboardApi()
      const text = allowed ? sync.takePending() : null
      if (!clipboard || text === null) return
      void write(clipboard, sync, text).then((done) => {
        if (done) onCopied()
      })
    },
  }
}
