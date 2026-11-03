/** `value` unless it is missing or empty (self-reported fields arrive as `""` too). */
export function nonBlank(value: string | null | undefined): string | undefined {
  return value === null || value === undefined || value === '' ? undefined : value
}

/** A unix-millisecond timestamp as a short local date and time, in `language`. */
export function formatDateTime(ms: number, language: string): string {
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(
    new Date(ms),
  )
}
