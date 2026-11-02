/** ⌘K on macOS, Ctrl+K elsewhere; both work everywhere. */
export function isPaletteShortcut(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'k'
}

export const IS_APPLE_PLATFORM = /mac|iphone|ipad/i.test(navigator.userAgent)
