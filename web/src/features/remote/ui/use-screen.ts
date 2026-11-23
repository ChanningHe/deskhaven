import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import type { RefObject } from 'react'

import { CssCursors } from '../cursor/css-cursor'
import type { CursorShape } from '../cursor/cursor'
import { localCursor, paintedCursor, type CursorScene } from '../cursor/plan'
import { pointerInput, wheelInput } from '../input/dom'
import type { Session } from '../session/session'
import { canvasPoint, drawCursor, drawFrame, type Rect, type Size } from './draw'
import { useStore } from './use-remote'

/** Draws the session's frames on `canvas`; returns the frame size once the first one is drawn. */
export function useVideo(session: Session, canvas: RefObject<HTMLCanvasElement | null>) {
  const [size, setSize] = useState<Size | null>(null)
  useEffect(() => {
    let stopped = false
    const run = async () => {
      for await (const frame of session.frames) {
        try {
          const target = stopped ? null : canvas.current
          const resized = target ? drawFrame(target, frame) : null
          if (resized) setSize(resized)
        } finally {
          frame.close()
        }
      }
    }
    void run()
    return () => {
      stopped = true
    }
  }, [session, canvas])
  return size
}

/** The content size of `element`, followed as it changes. */
export function useElementSize(element: RefObject<HTMLElement | null>): Size | null {
  const [size, setSize] = useState<Size | null>(null)
  useEffect(() => {
    const target = element.current
    if (!target || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return
      const { width, height } = entry.contentRect
      setSize((current) =>
        current?.width === width && current.height === height ? current : { width, height },
      )
    })
    observer.observe(target)
    return () => {
      observer.disconnect()
    }
  }, [element])
  return size
}

function useCssCursor(session: Session, shape: CursorShape | null, scale: number | null) {
  const [, ready] = useReducer((n: number) => n + 1, 0)
  const cursors = useMemo(() => new CssCursors(session.cursorImages, ready), [session])
  useEffect(() => {
    if (shape && scale !== null) cursors.prepare(shape, scale)
  }, [cursors, shape, scale])
  return shape && scale !== null ? cursors.cursor(shape, scale) : null
}

interface PaintState {
  scene: CursorScene
  local: { x: number; y: number } | null
  remoteMoved: boolean
  drawn: Rect | null
}

function paintCursor(
  canvas: RefObject<HTMLCanvasElement | null>,
  session: Session,
  stateRef: RefObject<PaintState>,
) {
  const ctx = canvas.current?.getContext('2d')
  const state = stateRef.current
  if (!ctx) return
  const { shape, position } = session.cursor.get()
  const display = session.peer.get()?.display
  const where = paintedCursor(state.scene, state.remoteMoved)
  let at = where === 'local' ? state.local : null
  if (where === 'remote' && position && display) {
    at = { x: position.x - display.x, y: position.y - display.y }
  }
  state.drawn = drawCursor(ctx, shape, at, state.drawn)
}

export interface CursorOptions {
  showRemote: boolean
  viewOnly: boolean
  scale: number | null
}

export function useCursor(
  session: Session,
  canvas: RefObject<HTMLCanvasElement | null>,
  { showRemote, viewOnly, scale }: CursorOptions,
) {
  const subscribe = useCallback(
    (listener: () => void) => session.cursor.subscribe(listener),
    [session],
  )
  const shape = useSyncExternalStore(subscribe, () => session.cursor.get().shape)
  const peer = useStore(session.peer)
  const permissions = useStore(session.permissions)
  const css = useCssCursor(session, shape, scale)
  const scene: CursorScene = {
    embedded: peer?.cursorEmbedded ?? false,
    control: permissions.keyboard && !viewOnly,
    showRemote,
    hasShape: shape !== null,
    css,
  }

  const state = useRef<PaintState>({ scene, local: null, remoteMoved: false, drawn: null })
  const paint = useCallback(() => {
    paintCursor(canvas, session, state)
  }, [session, canvas])

  useLayoutEffect(() => {
    state.current.scene = scene
    paint()
  })

  useEffect(() => {
    let position = session.cursor.get().position
    return session.cursor.subscribe(() => {
      const view = session.cursor.get()
      if (view.position && view.position !== position) state.current.remoteMoved = true
      position = view.position
      paint()
    })
  }, [session, paint])

  const moveTo = useCallback(
    (point: { x: number; y: number }) => {
      state.current.local = point
      state.current.remoteMoved = false
      paint()
    },
    [paint],
  )
  return { cursor: localCursor(scene), moveTo }
}

/**
 * Captures mouse and wheel on the screen canvas and sends them to the peer. A press focuses
 * `keys`, the screen's keyboard input, which follows the pointer so that an IME opens its
 * candidate window where the user is.
 */
export function useScreenInput(
  session: Session,
  canvas: RefObject<HTMLCanvasElement | null>,
  keys: RefObject<HTMLTextAreaElement | null>,
  onPointer: (point: { x: number; y: number }) => void,
) {
  useEffect(() => {
    const view = canvas.current
    const area = keys.current
    if (!view || !area) return
    const onPointerEvent = (e: PointerEvent) => {
      if (e.type === 'pointerdown') {
        area.focus({ preventScroll: true })
        view.setPointerCapture(e.pointerId)
      }
      e.preventDefault()
      const box = view.getBoundingClientRect()
      area.style.left = `${e.clientX - box.left}px`
      area.style.top = `${e.clientY - box.top}px`
      const point = canvasPoint(view, box, e.clientX, e.clientY)
      if (point) onPointer(point)
      const input = pointerInput(e, view)
      if (input) session.input(input)
    }
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      session.input(wheelInput(e))
    }
    const onContextMenu = (e: Event) => {
      e.preventDefault()
    }
    const listeners: [string, EventListener, AddEventListenerOptions?][] = [
      ['pointerdown', onPointerEvent as EventListener],
      ['pointermove', onPointerEvent as EventListener],
      ['pointerup', onPointerEvent as EventListener],
      ['wheel', onWheel as EventListener, { passive: false }],
      ['contextmenu', onContextMenu],
    ]
    for (const [type, listener, options] of listeners)
      view.addEventListener(type, listener, options)
    return () => {
      for (const [type, listener] of listeners) view.removeEventListener(type, listener)
    }
  }, [session, canvas, keys, onPointer])
}
