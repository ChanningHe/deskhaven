import type { ClipboardAccess } from './use-clipboard-sync'

export interface BrowserAbilities {
  keyboardLock: boolean
  /** Read access to the local clipboard; null while the browser has not answered. */
  clipboard: ClipboardAccess | null
}

const CLIPBOARD_LINES = {
  granted: 'remote.limits.clipboardBoth',
  prompt: 'remote.limits.clipboardAsk',
  denied: 'remote.limits.clipboardDenied',
  unsupported: 'remote.limits.clipboardUnsupported',
} as const satisfies Record<ClipboardAccess, string>

type LimitLine =
  | 'remote.limits.shortcutsFullscreen'
  | 'remote.limits.shortcutsMenu'
  | 'remote.limits.escape'
  | (typeof CLIPBOARD_LINES)[ClipboardAccess]

export function limitHints({ keyboardLock, clipboard }: BrowserAbilities): LimitLine[] {
  const keys: LimitLine[] = keyboardLock
    ? ['remote.limits.shortcutsFullscreen', 'remote.limits.escape']
    : ['remote.limits.shortcutsMenu']
  return [...keys, CLIPBOARD_LINES[clipboard ?? 'prompt']]
}
