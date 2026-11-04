// URL state of the audit page: the tab, the table keys, the filters of the tab and the open
// drawer (`item`). The filter keys are the admin API's query parameters (08 §6), so the
// validated search is also the request.

import type {
  ListAlarmAuditParams,
  ListConnectionAuditParams,
  ListConsoleAuditParams,
  ListFileAuditParams,
  ListLoginAuditParams,
} from '@/api/generated/model'
import { parseTableSearch, toListParams } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

/** Only the creation time orders the audit lists. */
export const AUDIT_SORTABLE = ['created_at'] as const

export const AUDIT_TABS = ['connections', 'files', 'alarms', 'logins', 'console'] as const
export type AuditTab = (typeof AUDIT_TABS)[number]

export const LOGIN_METHODS = ['password', 'totp', 'email_code', 'oidc', 'ldap', 'recovery'] as const
export const LOGIN_CLIENTS = ['rustdesk', 'console', 'api'] as const
export const ACTOR_KINDS = ['user', 'api_token', 'cli', 'system'] as const
export const DIRECTIONS = ['send', 'receive'] as const
/** The outcome labels the login flows write (`login_events.result`). */
export const LOGIN_RESULTS = [
  'success',
  'invalid_credentials',
  'disabled',
  'unverified',
  'locked',
  'tfa_required',
  'tfa_setup_required',
  'invalid_code',
  'invalid_challenge',
  'oidc_denied',
  'oidc_unavailable',
  'oidc_invalid_token',
  'no_linked_account',
  'provisioning_failed',
] as const

/** Filters of the list endpoints other than the time window. */
interface TabFilters {
  device?: string | undefined
  user?: number | undefined
  conn_type?: number | undefined
  direction?: (typeof DIRECTIONS)[number] | undefined
  typ?: number | undefined
  method?: (typeof LOGIN_METHODS)[number] | undefined
  client?: (typeof LOGIN_CLIENTS)[number] | undefined
  result?: (typeof LOGIN_RESULTS)[number] | undefined
  actor_kind?: (typeof ACTOR_KINDS)[number] | undefined
  action?: string | undefined
  target_type?: string | undefined
}

export type FilterKey = keyof TabFilters

export interface AuditSearch extends TableSearch, TabFilters {
  /** Absent: connections. */
  tab?: AuditTab | undefined
  /** Created at or after (unix ms). */
  from?: number | undefined
  /** Created before (unix ms). */
  to?: number | undefined
  /** Id of the record shown in the drawer. */
  item?: number | undefined
}

/** Which filters each tab has; anything else in the URL is dropped. */
export const TAB_FILTER_KEYS: Record<AuditTab, readonly FilterKey[]> = {
  connections: ['device', 'user', 'conn_type'],
  files: ['device', 'direction'],
  alarms: ['device', 'typ'],
  logins: ['user', 'method', 'client', 'result'],
  console: ['user', 'actor_kind', 'action', 'target_type'],
}

const MAX_TEXT = 256
const MAX_CODE = 1000

function toNumber(value: unknown): number | undefined {
  const number = typeof value === 'string' && value.trim() !== '' ? Number(value) : value
  return typeof number === 'number' && Number.isSafeInteger(number) && number >= 0
    ? number
    : undefined
}

function positive(value: unknown): number | undefined {
  const number = toNumber(value)
  return number !== undefined && number > 0 ? number : undefined
}

function code(value: unknown): number | undefined {
  const number = toNumber(value)
  return number !== undefined && number <= MAX_CODE ? number : undefined
}

/** The router reads `device=100` as the number 100: a number is the text it was typed as. */
function text(value: unknown): string | undefined {
  const raw = typeof value === 'number' && Number.isSafeInteger(value) ? String(value) : value
  const trimmed = typeof raw === 'string' ? raw.trim().slice(0, MAX_TEXT) : ''
  return trimmed === '' ? undefined : trimmed
}

function oneOf<T extends string>(values: readonly T[], value: unknown): T | undefined {
  return values.find((candidate) => candidate === value)
}

function parseFilters(raw: Record<string, unknown>): TabFilters {
  return {
    device: text(raw.device),
    user: positive(raw.user),
    conn_type: code(raw.conn_type),
    direction: oneOf(DIRECTIONS, raw.direction),
    typ: code(raw.typ),
    method: oneOf(LOGIN_METHODS, raw.method),
    client: oneOf(LOGIN_CLIENTS, raw.client),
    result: oneOf(LOGIN_RESULTS, raw.result),
    actor_kind: oneOf(ACTOR_KINDS, raw.actor_kind),
    action: text(raw.action),
    target_type: text(raw.target_type),
  }
}

function tabOf(search: AuditSearch): AuditTab {
  return search.tab ?? 'connections'
}

/** The filters of `tab` among `filters`, the rest dropped. */
function forTab(tab: AuditTab, filters: TabFilters): TabFilters {
  const allowed = TAB_FILTER_KEYS[tab]
  return Object.fromEntries(
    Object.entries(filters).filter(([key]) => allowed.includes(key as FilterKey)),
  )
}

/** Validates the raw URL search; anything malformed is dropped. */
export function parseAuditSearch(raw: Record<string, unknown>): AuditSearch {
  const tab = oneOf(AUDIT_TABS, raw.tab)
  const shown = tab ?? 'connections'
  return {
    ...parseTableSearch(raw, { sortable: AUDIT_SORTABLE }),
    ...forTab(shown, parseFilters(raw)),
    tab: tab === 'connections' ? undefined : tab,
    from: toNumber(raw.from),
    to: toNumber(raw.to),
    item: positive(raw.item),
  }
}

/** The tab's own filters in effect (the time window is counted by the caller). */
export function activeFilters(search: AuditSearch): number {
  const keys = TAB_FILTER_KEYS[tabOf(search)]
  return keys.filter((key) => search[key] !== undefined).length
}

/** Whether any filter other than `q` is set. */
export function hasAuditFilters(search: AuditSearch): boolean {
  return activeFilters(search) > 0 || search.from !== undefined || search.to !== undefined
}

/** A search that keeps the tab's filters and clears them. */
export function clearedFilters(search: AuditSearch): Partial<AuditSearch> {
  const cleared = Object.fromEntries(TAB_FILTER_KEYS[tabOf(search)].map((key) => [key, undefined]))
  return { ...cleared, q: undefined, from: undefined, to: undefined }
}

/** A list request: every endpoint takes the keys of its own filters and ignores none. */
export type AuditListParams = ListConnectionAuditParams &
  ListFileAuditParams &
  ListAlarmAuditParams &
  ListLoginAuditParams &
  ListConsoleAuditParams

/**
 * The list request for a search: the table keys, the time window and the tab's own
 * filters; keys the endpoint of the tab does not have are absent.
 */
export function auditListParams(search: AuditSearch): AuditListParams {
  const filters = forTab(tabOf(search), search)
  const window = { from: search.from, to: search.to }
  const defined = Object.entries({ ...filters, ...window }).filter(
    ([, value]) => value !== undefined,
  )
  return { ...toListParams(search), ...Object.fromEntries(defined) }
}
