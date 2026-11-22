import { useCallback, useEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'

import { pointerInput, wheelInput } from '../input/dom'
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
