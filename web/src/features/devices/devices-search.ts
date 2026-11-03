// URL state of the devices page: the table keys plus the list filters (the admin API's
// query parameters, 08 §6) and the open drawer (`device`).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'
import type { DeviceStatusView, ListDevicesParams } from '@/api/generated/model'

export const DEVICE_SORTABLE = ['rustdesk_id', 'hostname', 'os', 'version', 'last_seen_at'] as const

export const DEVICE_STATUSES: readonly DeviceStatusView[] = ['active', 'disabled']

export interface DevicesSearch extends TableSearch {
  online?: boolean | undefined
  device_group?: number | undefined
  status?: DeviceStatusView | undefined
  /** Id of the device shown in the drawer. */
  device?: number | undefined
}

function positiveId(value: unknown): number | undefined {
  const number = typeof value === 'string' ? Number(value) : value
  return typeof number === 'number' && Number.isSafeInteger(number) && number > 0
    ? number
    : undefined
}

/** The router parses `true` / `false` into booleans; a hand-typed URL may leave strings. */
function parseBoolean(value: unknown): boolean | undefined {
  if (value === true || value === 'true') return true
  if (value === false || value === 'false') return false
  return undefined
}

/** Validates the raw URL search; anything malformed is dropped. */
export function parseDevicesSearch(raw: Record<string, unknown>): DevicesSearch {
  return {
    ...parseTableSearch(raw, { sortable: DEVICE_SORTABLE }),
    online: parseBoolean(raw.online),
    device_group: positiveId(raw.device_group),
    status: DEVICE_STATUSES.find((status) => status === raw.status),
    device: positiveId(raw.device),
  }
}

/** Whether any page-specific filter is set (`q` is detected by the table). */
export function hasDeviceFilters(search: DevicesSearch): boolean {
  return (
    search.online !== undefined || search.device_group !== undefined || search.status !== undefined
  )
}

/** The list request for a search (the drawer id is not a filter). */
export function devicesListParams(
  search: DevicesSearch,
  base: ListDevicesParams,
): ListDevicesParams {
  return {
    ...base,
    ...(search.online === undefined ? {} : { online: search.online }),
    ...(search.device_group === undefined ? {} : { device_group: search.device_group }),
    ...(search.status === undefined ? {} : { status: search.status }),
  }
}
