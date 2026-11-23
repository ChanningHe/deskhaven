import { useEffect } from 'react'

interface KeyboardLock {
  lock(): Promise<void>
  unlock(): void
}

function keyboardLock(): KeyboardLock | null {
  const keyboard: Partial<KeyboardLock> | undefined = (
    navigator as { keyboard?: Partial<KeyboardLock> }
  ).keyboard
  return typeof keyboard?.lock === 'function' && typeof keyboard.unlock === 'function'
    ? (keyboard as KeyboardLock)
    : null
}

export function canLockKeyboard(): boolean {
  return keyboardLock() !== null
}

export function useKeyboardLock(fullscreen: boolean): void {
  useEffect(() => {
    const keyboard = keyboardLock()
    if (!fullscreen || !keyboard) return
    // A refused lock (page not focused) leaves the shortcuts with the browser; the send keys menu covers them.
    keyboard.lock().catch(() => undefined)
    return () => {
      keyboard.unlock()
    }
  }, [fullscreen])
}
