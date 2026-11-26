/** The Clipboard API exists only in secure contexts and can be denied. */
export async function copyToClipboard(text: string): Promise<boolean> {
  if (typeof navigator.clipboard === 'undefined') return false
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}
