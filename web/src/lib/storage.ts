// Guarded localStorage access. Storage can be unavailable (privacy mode,
// disabled cookies, quota exceeded); preferences are best-effort, so failures
// degrade to "not persisted" instead of crashing the console.

export function readStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

export function writeStorage(key: string, value: string): boolean {
  try {
    window.localStorage.setItem(key, value)
    return true
  } catch {
    return false
  }
}
