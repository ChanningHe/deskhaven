import { useCallback, useEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'

import { keyInput, pointerInput, wheelInput } from '../input/dom'
import type { Session } from '../session/session'
import { canvasPoint, drawCursor, drawFrame, type Size } from './draw'

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

/**
 * Paints the peer's cursor on the overlay: where the local pointer is while it moves over
 * the screen, and where the peer reports it when it moves there (someone else moved it).
 * Returns the function the pointer handler calls with canvas pixels.
 */
export function useCursor(session: Session, canvas: RefObject<HTMLCanvasElement | null>) {
  const at = useRef<{ x: number; y: number } | null>(null)
  const paint = useCallback(() => {
    const ctx = canvas.current?.getContext('2d')
    if (ctx) drawCursor(ctx, session.cursor.get().shape, at.current)
  }, [session, canvas])

  useEffect(() => {
    let position = session.cursor.get().position
    return session.cursor.subscribe(() => {
      const view = session.cursor.get()
      const display = session.peer.get()?.display
      if (view.position && view.position !== position && display) {
        at.current = { x: view.position.x - display.x, y: view.position.y - display.y }
      }
      position = view.position
      paint()
    })
  }, [session, paint])

  const moveTo = useCallback(
    (point: { x: number; y: number }) => {
      at.current = point
      paint()
    },
    [paint],
  )
  return { paint, moveTo }
}

// Keys the browser keeps: full screen and the developer tools. Everything else the focused
// screen takes, so the page's own shortcuts (find, save, back) do not fire.
const BROWSER_KEYS: ReadonlySet<string> = new Set(['F11', 'F12'])

/** Captures keyboard, mouse and wheel on the focused screen canvas and sends them to the peer. */
export function useScreenInput(
  session: Session,
  canvas: RefObject<HTMLCanvasElement | null>,
  onPointer: (point: { x: number; y: number }) => void,
) {
  useEffect(() => {
    const view = canvas.current
    if (!view) return
    const onPointerEvent = (e: PointerEvent) => {
      if (e.type === 'pointerdown') {
        view.focus()
        view.setPointerCapture(e.pointerId)
      }
      e.preventDefault()
      const point = canvasPoint(view, view.getBoundingClientRect(), e.clientX, e.clientY)
      if (point) onPointer(point)
      const input = pointerInput(e, view)
      if (input) session.input(input)
    }
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      session.input(wheelInput(e))
    }
    const onKey = (e: KeyboardEvent) => {
      if (BROWSER_KEYS.has(e.code) || e.isComposing) return
      e.preventDefault()
      session.input(keyInput(e))
    }
    const onBlur = () => {
      session.input({ kind: 'release' })
    }
    const onContextMenu = (e: Event) => {
      e.preventDefault()
    }
    const listeners: [string, EventListener, AddEventListenerOptions?][] = [
      ['pointerdown', onPointerEvent as EventListener],
      ['pointermove', onPointerEvent as EventListener],
      ['pointerup', onPointerEvent as EventListener],
      ['wheel', onWheel as EventListener, { passive: false }],
      ['keydown', onKey as EventListener],
      ['keyup', onKey as EventListener],
      ['blur', onBlur],
      ['contextmenu', onContextMenu],
    ]
    for (const [type, listener, options] of listeners)
      view.addEventListener(type, listener, options)
    return () => {
      for (const [type, listener] of listeners) view.removeEventListener(type, listener)
    }
  }, [session, canvas, onPointer])
}
