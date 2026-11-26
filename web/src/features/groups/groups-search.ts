import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const GROUP_SORTABLE = ['name', 'created_at'] as const

export type GroupsTab = 'users' | 'devices'

export interface GroupsSearch extends TableSearch {
  tab?: 'devices' | undefined
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
