import type { ClipboardAccess } from './use-clipboard-sync'

export interface BrowserAbilities {
  keyboardLock: boolean
  clipboard: ClipboardAccess | null
}

const CLIPBOARD_LINES = {
  granted: 'remote.limits.clipboardBoth',
  prompt: 'remote.limits.clipboardAsk',
  denied: 'remote.limits.clipboardDenied',
  unsupported: null,
} as const satisfies Record<ClipboardAccess, string | null>

type LimitLine =
  | 'remote.limits.shortcutsFullscreen'
  | 'remote.limits.shortcutsMenu'
  | 'remote.limits.escape'
  | NonNullable<(typeof CLIPBOARD_LINES)[ClipboardAccess]>

export function limitHints({ keyboardLock, clipboard }: BrowserAbilities): LimitLine[] {
  const keys: LimitLine[] = keyboardLock
    ? ['remote.limits.shortcutsFullscreen', 'remote.limits.escape']
    : ['remote.limits.shortcutsMenu']
  const line = CLIPBOARD_LINES[clipboard ?? 'prompt']
  return line ? [...keys, line] : keys
}
