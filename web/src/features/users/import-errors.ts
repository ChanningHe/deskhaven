import { isApiError } from '@/api/api-error'
import type { ImportRowError } from '@/api/generated/model'

const HTTP_UNPROCESSABLE = 422

function isRowError(value: unknown): value is ImportRowError {
  if (typeof value !== 'object' || value === null) return false
  const row = value as Record<string, unknown>
  return (
    typeof row.line === 'number' &&
    typeof row.code === 'string' &&
    (row.field === null || row.field === undefined || typeof row.field === 'string')
  )
}

/**
 * The per-row problems of a rejected import (`details.rows` of a 422, see the
 * `ImportRejected` schema); empty for any other error.
 */
export function importRowErrors(error: unknown): ImportRowError[] {
  if (!isApiError(error) || error.status !== HTTP_UNPROCESSABLE) return []
  const rows = error.details?.rows
  return Array.isArray(rows) ? rows.filter(isRowError) : []
}
