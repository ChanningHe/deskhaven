/**
 * ⌘K on macOS, Ctrl+K elsewhere; both work everywhere. Matched on the physical
 * key (`code`), so it also works with non-Latin keyboard layouts.
 */
export function isPaletteShortcut(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && !event.altKey && event.code === 'KeyK'
}

/** Whether a modal dialog (other than the palette) is open: the shortcut then does nothing. */
export function isOtherDialogOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"]') !== null
}

export const IS_APPLE_PLATFORM = /mac|iphone|ipad/i.test(navigator.userAgent)
