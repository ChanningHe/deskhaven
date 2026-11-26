import { isApiError } from '@/api/api-error'
import type { SettingView } from '@/api/generated/model'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isSettingView(value: unknown): value is SettingView {
  return (
    isRecord(value) &&
    typeof value.key === 'string' &&
    typeof value.version === 'number' &&
    isRecord(value.schema) &&
    isRecord(value.source)
  )
}

export function conflictsOf(error: unknown): SettingView[] {
  if (!isApiError(error) || error.code !== 'version_conflict') return []
  const conflicts = error.details?.conflicts
  return Array.isArray(conflicts) ? conflicts.filter(isSettingView) : []
}

export function fieldProblemsOf(error: unknown): Readonly<Record<string, readonly string[]>> {
  if (!isApiError(error) || error.code !== 'validation_failed') return {}
  const fields = error.details?.fields
  if (!isRecord(fields)) return {}
  return Object.fromEntries(
    Object.entries(fields).map(([key, reasons]) => [
      key,
      Array.isArray(reasons) ? reasons.filter((r): r is string => typeof r === 'string') : [],
    ]),
  )
}
