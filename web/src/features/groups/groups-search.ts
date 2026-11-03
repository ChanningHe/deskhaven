// URL state of the groups page: the tab (user groups, or device groups with `tab=devices`),
// the table keys and the open drawer (`group`, an id of the active tab's kind).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const GROUP_SORTABLE = ['name', 'created_at'] as const

export type GroupsTab = 'users' | 'devices'

export interface GroupsSearch extends TableSearch {
  /** Device groups; absent: user groups. */
  tab?: 'devices' | undefined
  /** Id of the group shown in the drawer. */
  group?: number | undefined
}

export function parseGroupsSearch(raw: Record<string, unknown>): GroupsSearch {
  const group = typeof raw.group === 'string' ? Number(raw.group) : raw.group
  return {
    ...parseTableSearch(raw, { sortable: GROUP_SORTABLE }),
    tab: raw.tab === 'devices' ? 'devices' : undefined,
    group:
      typeof group === 'number' && Number.isSafeInteger(group) && group > 0 ? group : undefined,
  }
}
