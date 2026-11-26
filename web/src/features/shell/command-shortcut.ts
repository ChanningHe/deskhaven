/** Matched on the physical key (`code`), so it also works with non-Latin keyboard layouts. */
export function isPaletteShortcut(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && !event.altKey && event.code === 'KeyK'
}

export function isOtherDialogOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"]') !== null
}

export const IS_APPLE_PLATFORM = /mac|iphone|ipad/i.test(navigator.userAgent)
