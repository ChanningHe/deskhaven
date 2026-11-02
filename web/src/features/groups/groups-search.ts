// URL state of the groups page: the table keys and the open drawer (`group`).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const GROUP_SORTABLE = ['name', 'created_at'] as const

export interface GroupsSearch extends TableSearch {
  /** Id of the group shown in the drawer. */
  group?: number | undefined
}

export function parseGroupsSearch(raw: Record<string, unknown>): GroupsSearch {
  const group = typeof raw.group === 'string' ? Number(raw.group) : raw.group
  return {
    ...parseTableSearch(raw, { sortable: GROUP_SORTABLE }),
    group:
      typeof group === 'number' && Number.isSafeInteger(group) && group > 0 ? group : undefined,
  }
}
