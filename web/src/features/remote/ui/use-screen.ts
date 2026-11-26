import {
  useCallback,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import type { RefObject } from 'react'

import type { RemotePhase } from '../connect/controller'
import { CssCursors } from '../cursor/css-cursor'
import type { CursorShape } from '../cursor/cursor'
import { localCursor, paintedCursor, type CursorScene } from '../cursor/plan'
import { pointerInput, wheelInput } from '../input/dom'
import type { Session, SessionState } from '../session/session'
import { canvasPoint, drawCursor, drawFrame, type Rect, type Size } from './draw'
import { useStore } from './use-remote'

function snapshot(source: HTMLCanvasElement): HTMLCanvasElement | null {
  const copy = document.createElement('canvas')
  copy.width = source.width
  copy.height = source.height
  const ctx = copy.getContext('2d')
  if (!ctx) return null
  ctx.drawImage(source, 0, 0)
  copy.setAttribute('aria-hidden', 'true')
  return copy
}

/** `onFrozen` hears null on the first frame drawn, and a copy of the last one when drawing stops. */
export function useVideo(
  session: Session,
  canvas: RefObject<HTMLCanvasElement | null>,
  onFrozen?: (frame: HTMLCanvasElement | null) => void,
) {
  const [size, setSize] = useState<Size | null>(null)
  const frozen = useEffectEvent((frame: HTMLCanvasElement | null) => {
    onFrozen?.(frame)
  })
  // A layout effect: the copy must reach the page before the next paint, or the full-page
  // reconnect panel flashes.
  useLayoutEffect(() => {
    const target = canvas.current
    let stopped = false
    let drawn = false
    const run = async () => {
      for await (const frame of session.frames) {
        try {
          if (stopped || !target) continue
          const resized = drawFrame(target, frame)
          if (resized) setSize(resized)
          if (!drawn) frozen(null)
          drawn = true
        } finally {
          frame.close()
        }
      }
    }
    void run()
    return () => {
      stopped = true
      const copy = drawn && target ? snapshot(target) : null
      if (copy) frozen(copy)
    }
  }, [session, canvas])
  return size
}

export interface Frozen {
  canvas: HTMLCanvasElement
  hostname: string | undefined
}

export interface FrozenFrame {
  frozen: Frozen | null
  hold: (frozen: Frozen | null) => void
}

function useSessionKind(phase: RemotePhase): SessionState['kind'] | undefined {
  const session = phase.kind === 'session' ? phase.session : null
  const subscribe = useCallback(
    (listener: () => void) => (session ? session.state.subscribe(listener) : () => undefined),
    [session],
  )
  return useSyncExternalStore(subscribe, () => session?.state.get().kind)
}

export function useFrozenFrame(phase: RemotePhase): FrozenFrame {
  const [held, setHeld] = useState<Frozen | null>(null)
  const kind = useSessionKind(phase)
  const holds = phase.kind === 'reconnecting' || kind === 'active'
  const [holding, setHolding] = useState(holds)
  if (holds !== holding) {
    setHolding(holds)
    if (holds) setHeld(null)
  }
  return { frozen: holds ? held : null, hold: setHeld }
}

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

/** A press focuses `keys`, which follows the pointer so that an IME opens its candidate window there. */
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
