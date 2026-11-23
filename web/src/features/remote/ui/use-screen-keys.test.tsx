import { act, render, screen } from '@testing-library/react'
import { useRef } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FakeSession } from '../connect/fake-session'
import type { RemoteInput } from '../input/input'
import { useScreenKeys } from './use-screen-keys'

const FALLBACK_MS = 100

let sent: string[]
let session: FakeSession
let area: HTMLTextAreaElement
const onPaste = vi.fn<(text: string) => void>()
const onLeave = vi.fn<() => void>()

function Harness() {
  const ref = useRef<HTMLTextAreaElement>(null)
  useScreenKeys(session, ref, onLeave, onPaste)
  return <textarea ref={ref} aria-label="keys" />
}

function label(input: RemoteInput): string {
  return input.kind === 'key' ? `${input.down ? '+' : '-'}${input.key}` : input.kind
}

beforeEach(() => {
  vi.useFakeTimers()
  sent = []
  session = new FakeSession()
  session.activate()
  vi.spyOn(session, 'input').mockImplementation((input) => {
    sent.push(label(input))
  })
  onPaste.mockReset()
  onLeave.mockReset()
  onPaste.mockImplementation((text) => {
    sent.push(`clip:${text}`)
  })
  render(<Harness />)
  area = screen.getByRole('textbox')
  area.focus()
})

afterEach(() => {
  vi.useRealTimers()
})

type KeyInit = KeyboardEventInit & { key: string }

function key(type: 'keydown' | 'keyup', init: KeyInit): KeyboardEvent {
  const event = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init })
  act(() => {
    area.dispatchEvent(event)
  })
  return event
}

function paste(text: string | null): Event {
  const event = new Event('paste', { bubbles: true, cancelable: true })
  const clipboardData = { getData: (type: string) => (type === 'text/plain' ? (text ?? '') : '') }
  Object.defineProperty(event, 'clipboardData', { value: clipboardData })
  act(() => {
    area.dispatchEvent(event)
  })
  return event
}

const COMBOS: [string, KeyInit, KeyInit[]][] = [
  ['Ctrl+V', { key: 'v', code: 'KeyV', ctrlKey: true }, [{ key: 'Control', ctrlKey: true }]],
  ['Cmd+V', { key: 'v', code: 'KeyV', metaKey: true }, [{ key: 'Meta', metaKey: true }]],
  [
    'Shift+Insert',
    { key: 'Insert', code: 'Insert', shiftKey: true },
    [{ key: 'Shift', shiftKey: true }],
  ],
  [
    'Ctrl+Shift+V',
    { key: 'V', code: 'KeyV', ctrlKey: true, shiftKey: true },
    [
      { key: 'Control', ctrlKey: true },
      { key: 'Shift', ctrlKey: true, shiftKey: true },
    ],
  ],
]

describe('paste shortcuts', () => {
  it.each(COMBOS)('%s: the text goes to the peer before the key', (_, combo, modifiers) => {
    for (const modifier of modifiers) key('keydown', modifier)
    const down = key('keydown', combo)
    expect(down.defaultPrevented).toBe(false)
    expect(sent).not.toContain(`+${combo.key}`)

    const pasted = paste('from here')
    expect(pasted.defaultPrevented).toBe(true)
    key('keyup', combo)
    expect(sent.slice(modifiers.length)).toEqual([
      'clip:from here',
      `+${combo.key}`,
      `-${combo.key}`,
    ])
    expect(area).toHaveValue('')
  })

  it('holds the key up that comes before the paste event, in order', () => {
    key('keydown', { key: 'Control', ctrlKey: true })
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    const up = key('keyup', { key: 'v', code: 'KeyV', ctrlKey: true })
    key('keyup', { key: 'Control' })
    expect(up.defaultPrevented).toBe(true)
    expect(sent).toEqual(['+Control'])
    paste('text')
    expect(sent).toEqual(['+Control', 'clip:text', '+v', '-v', '-Control'])
  })

  it('lets the key through unchanged when the clipboard is empty', () => {
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    paste('')
    key('keyup', { key: 'v', code: 'KeyV', ctrlKey: true })
    expect(onPaste).not.toHaveBeenCalled()
    expect(sent).toEqual(['+v', '-v'])
  })

  it('lets the key through when no paste event comes', () => {
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    key('keyup', { key: 'v', code: 'KeyV', ctrlKey: true })
    expect(sent).toEqual([])
    act(() => {
      vi.advanceTimersByTime(FALLBACK_MS)
    })
    expect(sent).toEqual(['+v', '-v'])
    act(() => {
      vi.advanceTimersByTime(FALLBACK_MS * 5)
    })
    expect(sent).toEqual(['+v', '-v'])
  })

  it('sends the key once when the paste event comes after the wait is over', () => {
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    act(() => {
      vi.advanceTimersByTime(FALLBACK_MS)
    })
    const late = paste('late')
    expect(late.defaultPrevented).toBe(true)
    expect(sent).toEqual(['+v'])
  })

  it('does not run the fallback after the paste event', () => {
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    paste('text')
    act(() => {
      vi.advanceTimersByTime(FALLBACK_MS * 2)
    })
    expect(sent).toEqual(['clip:text', '+v'])
  })

  it('drops the held key when focus leaves, and lets go of the rest', () => {
    key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true })
    act(() => {
      area.blur()
      vi.advanceTimersByTime(FALLBACK_MS * 2)
    })
    expect(sent).toEqual(['release'])
  })

  it('keeps the text out of the field when it is pasted with no shortcut', () => {
    const pasted = paste('from a menu')
    expect(pasted.defaultPrevented).toBe(true)
    expect(onPaste).not.toHaveBeenCalled()
    expect(sent).toEqual([])
  })

  it.each<[string, KeyInit]>([
    ['V alone', { key: 'v', code: 'KeyV' }],
    ['Shift+V', { key: 'V', code: 'KeyV', shiftKey: true }],
    ['Alt+Ctrl+V', { key: 'v', code: 'KeyV', ctrlKey: true, altKey: true }],
    ['Ctrl+Cmd+V', { key: 'v', code: 'KeyV', ctrlKey: true, metaKey: true }],
    ['Cmd+Shift+V', { key: 'V', code: 'KeyV', metaKey: true, shiftKey: true }],
    ['Insert alone', { key: 'Insert', code: 'Insert' }],
    ['Ctrl+Insert', { key: 'Insert', code: 'Insert', ctrlKey: true }],
    ['Ctrl+C', { key: 'c', code: 'KeyC', ctrlKey: true }],
  ])('%s goes straight to the peer', (_, combo) => {
    const down = key('keydown', combo)
    expect(down.defaultPrevented).toBe(true)
    expect(sent).toEqual([`+${combo.key}`])
  })

  it('leaves a key an IME composes alone, Ctrl+V or not', () => {
    const start = key('keydown', { key: 'Process', code: 'KeyV', ctrlKey: true })
    const composing = key('keydown', { key: 'v', code: 'KeyV', ctrlKey: true, isComposing: true })
    expect(start.defaultPrevented).toBe(false)
    expect(composing.defaultPrevented).toBe(false)
    expect(sent).toEqual([])
    act(() => {
      vi.advanceTimersByTime(FALLBACK_MS * 2)
    })
    expect(sent).toEqual([])
  })

  it('still sends what an IME commits as text', () => {
    key('keydown', { key: 'Process', code: 'KeyN' })
    area.value = '你'
    act(() => {
      area.dispatchEvent(new CompositionEvent('compositionend', { data: '你' }))
    })
    expect(sent).toEqual(['text'])
    expect(area).toHaveValue('')
  })
})

describe('Escape while the browser holds the keyboard (full screen with Keyboard Lock)', () => {
  it('sends a short press to the peer, down and up, and stays on the screen', () => {
    const down = key('keydown', { key: 'Escape', code: 'Escape' })
    key('keyup', { key: 'Escape', code: 'Escape' })
    expect(down.defaultPrevented).toBe(true)
    expect(sent).toEqual(['+Escape', '-Escape'])
    expect(onLeave).not.toHaveBeenCalled()
  })

  it('does not leave the screen on the repeats of a held Escape, which ends full screen itself', () => {
    key('keydown', { key: 'Escape', code: 'Escape' })
    for (let i = 0; i < 5; i++) key('keydown', { key: 'Escape', code: 'Escape', repeat: true })
    expect(onLeave).not.toHaveBeenCalled()
    expect(sent.every((entry) => entry === '+Escape')).toBe(true)
    expect(sent).toHaveLength(6)
  })

  it('sends the Windows key to the peer as a key of its own', () => {
    const down = key('keydown', { key: 'Meta', code: 'MetaLeft', metaKey: true })
    key('keyup', { key: 'Meta', code: 'MetaLeft' })
    expect(down.defaultPrevented).toBe(true)
    expect(sent).toEqual(['+Meta', '-Meta'])
  })
})
