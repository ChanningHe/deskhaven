/** A unix-millisecond timestamp as a short local date and time, in `language`. */
export function formatDateTime(ms: number, language: string): string {
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(
    new Date(ms),
  )
}
