// The audit time window as the date fields show it. The URL and the API carry unix
// milliseconds: `from` (inclusive) is the start of the first day, `to` (exclusive) the
// start of the day after the last one, both in the browser's time zone.

const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/

function localDate(value: string, dayOffset: number): Date | undefined {
  const match = DATE_PATTERN.exec(value)
  if (match === null) return undefined
  const [, year, month, day] = match.map(Number)
  if (year === undefined || month === undefined || day === undefined) return undefined
  const date = new Date(year, month - 1, day + dayOffset)
  // `new Date` rolls 2026-02-31 over to March: reject what does not round-trip.
  const real = new Date(year, month - 1, day)
  return real.getMonth() === month - 1 ? date : undefined
}

/** Start of the day `value` (`YYYY-MM-DD`) in ms, or undefined for anything else. */
export function windowStart(value: string): number | undefined {
  return localDate(value, 0)?.getTime()
}

/** Start of the day after `value` in ms: the exclusive end of a window ending that day. */
export function windowEnd(value: string): number | undefined {
  return localDate(value, 1)?.getTime()
}

function pad(number: number, width: number): string {
  return String(number).padStart(width, '0')
}

function dateField(date: Date): string {
  return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1, 2)}-${pad(date.getDate(), 2)}`
}

/** The date field value for a window start. */
export function startField(ms: number | undefined): string {
  return ms === undefined ? '' : dateField(new Date(ms))
}

/** The date field value for a window end: the last day inside it. */
export function endField(ms: number | undefined): string {
  return ms === undefined ? '' : dateField(new Date(ms - 1))
}
