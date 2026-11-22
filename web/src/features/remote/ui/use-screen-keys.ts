import { useEffect, useState } from 'react'
import type { RefObject } from 'react'

import { keyInput, textInput } from '../input/dom'
import type { Session } from '../session/session'

// Keys the browser keeps: full screen and the developer tools. Everything else the focused
// screen takes, so the page's own shortcuts (find, save, back) do not fire.
const BROWSER_KEYS: ReadonlySet<string> = new Set(['F11', 'F12'])

/** A second Escape within this time leaves the screen; the first one still reaches the peer. */
const LEAVE_ESCAPE_MS = 500

/** How long the hint on leaving the screen shows after it takes focus. */
const HINT_MS = 4000

/**
 * The key belongs to an IME: it composes, or starts to ('Process'). Its default must run for
 * the IME to see it, and the result arrives as compositionend.
 */
function imeKey(e: KeyboardEvent): boolean {
  return e.isComposing || e.key === 'Process'
}

function plainEscape(e: KeyboardEvent): boolean {
  return (
    e.type === 'keydown' &&
    e.key === 'Escape' &&
    !e.repeat &&
    !(e.altKey || e.ctrlKey || e.metaKey || e.shiftKey)
  )
}

/**
 * Sends the keys typed into `input`, the screen's hidden text field, to the peer. Composed text
 * goes as text. Escape twice in quick succession leaves the screen through `onLeave` (Tab stays
 * with the peer). Returns whether the hint on leaving is showing.
 */
export function useScreenKeys(
  session: Session,
  input: RefObject<HTMLTextAreaElement | null>,
  onLeave: () => void,
): boolean {
  const [hint, setHint] = useState(false)
  useEffect(() => {
    if (!hint) return
    const timer = setTimeout(() => {
      setHint(false)
    }, HINT_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [hint])

  useEffect(() => {
    const area = input.current
    if (!area) return
    let lastEscape: number | null = null
    const leaves = (e: KeyboardEvent): boolean => {
      if (e.type !== 'keydown') return false
      const previous = lastEscape
      lastEscape = plainEscape(e) ? e.timeStamp : null
      if (lastEscape === null || previous === null) return false
      if (e.timeStamp - previous > LEAVE_ESCAPE_MS) return false
      lastEscape = null
      return true
    }
    const onKey = (e: KeyboardEvent) => {
      if (BROWSER_KEYS.has(e.code) || imeKey(e)) return
      e.preventDefault()
      if (!leaves(e)) {
        session.input(keyInput(e))
        return
      }
      area.blur()
      onLeave()
    }
    const onCompositionEnd = (e: CompositionEvent) => {
      session.input(textInput(e))
      area.value = ''
    }
    // Only an IME writes into the field; whatever else lands there is not kept.
    const onInput = (e: Event) => {
      if (!(e as InputEvent).isComposing) area.value = ''
    }
    const onFocus = () => {
      setHint(true)
    }
    const onBlur = () => {
      lastEscape = null
      setHint(false)
      session.input({ kind: 'release' })
    }
    const listeners: [string, EventListener][] = [
      ['keydown', onKey as EventListener],
      ['keyup', onKey as EventListener],
      ['compositionend', onCompositionEnd as EventListener],
      ['input', onInput],
      ['focus', onFocus],
      ['blur', onBlur],
    ]
    for (const [type, listener] of listeners) area.addEventListener(type, listener)
    return () => {
      for (const [type, listener] of listeners) area.removeEventListener(type, listener)
    }
  }, [session, input, onLeave])

  return hint
}
