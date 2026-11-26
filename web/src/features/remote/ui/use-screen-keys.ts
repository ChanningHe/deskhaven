import { useEffect, useEffectEvent, useState } from 'react'
import type { RefObject } from 'react'

import { keyInput, textInput } from '../input/dom'
import type { RemoteInput } from '../input/input'
import type { Session } from '../session/session'

// Keys the browser keeps: full screen and the developer tools. Everything else the focused
// screen takes, so the page's own shortcuts (find, save, back) do not fire.
const BROWSER_KEYS: ReadonlySet<string> = new Set(['F11', 'F12'])

/** A second Escape within this time leaves the screen; the first one still reaches the peer. */
const LEAVE_ESCAPE_MS = 500

const HINT_MS = 4000

/**
 * The key belongs to an IME: it composes, or starts to ('Process'). Its default must run for
 * the IME to see it, and the result arrives as compositionend.
 */
function imeKey(e: KeyboardEvent): boolean {
  return e.isComposing || e.key === 'Process'
}

const PASTE_WAIT_MS = 100

export function pasteKey(e: KeyboardEvent): boolean {
  if (e.type !== 'keydown' || e.altKey) return false
  if (e.key === 'Insert') return e.shiftKey && !e.ctrlKey && !e.metaKey
  if (e.key.toLowerCase() !== 'v') return false
  return e.metaKey ? !(e.ctrlKey || e.shiftKey) : e.ctrlKey
}

function plainEscape(e: KeyboardEvent): boolean {
  return (
    e.type === 'keydown' &&
    e.key === 'Escape' &&
    !e.repeat &&
    !(e.altKey || e.ctrlKey || e.metaKey || e.shiftKey)
  )
}

/** Escape twice in quick succession leaves the screen through `onLeave`; Tab stays with the peer. */
export function useScreenKeys(
  session: Session,
  input: RefObject<HTMLTextAreaElement | null>,
  onLeave: () => void,
  onPaste: (text: string) => void,
): boolean {
  const [hint, setHint] = useState(false)
  const paste = useEffectEvent(onPaste)
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
    let held: { inputs: RemoteInput[]; timer: ReturnType<typeof setTimeout> } | null = null
    const letGo = () => {
      if (!held) return
      clearTimeout(held.timer)
      const { inputs } = held
      held = null
      for (const item of inputs) session.input(item)
    }
    const drop = () => {
      if (held) clearTimeout(held.timer)
      held = null
    }
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
      if (held) {
        e.preventDefault()
        held.inputs.push(keyInput(e))
        return
      }
      if (pasteKey(e)) {
        held = { inputs: [keyInput(e)], timer: setTimeout(letGo, PASTE_WAIT_MS) }
        return
      }
      e.preventDefault()
      if (!leaves(e)) {
        session.input(keyInput(e))
        return
      }
      area.blur()
      onLeave()
    }
    // The peer applies the clipboard and the key on different threads; sent back to back, the new
    // text still won 20 of 20 pastes.
    const onPasteEvent = (e: ClipboardEvent) => {
      e.preventDefault()
      if (!held) return
      try {
        const text = e.clipboardData?.getData('text/plain') ?? ''
        if (text !== '') paste(text)
      } finally {
        letGo()
      }
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
      drop()
      lastEscape = null
      setHint(false)
      session.input({ kind: 'release' })
    }
    const listeners: [string, EventListener][] = [
      ['keydown', onKey as EventListener],
      ['keyup', onKey as EventListener],
      ['paste', onPasteEvent as EventListener],
      ['compositionend', onCompositionEnd as EventListener],
      ['input', onInput],
      ['focus', onFocus],
      ['blur', onBlur],
    ]
    for (const [type, listener] of listeners) area.addEventListener(type, listener)
    return () => {
      drop()
      for (const [type, listener] of listeners) area.removeEventListener(type, listener)
    }
  }, [session, input, onLeave])

  return hint
}
