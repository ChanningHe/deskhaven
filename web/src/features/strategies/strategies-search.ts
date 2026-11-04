// URL state of the strategies page: the table keys, the open drawer (`strategy`, an id) and
// the drawer's tab (`tab`; absent: the options).

import { parseTableSearch } from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'

export const STRATEGY_SORTABLE = ['name', 'updated_at'] as const

export const STRATEGY_TABS = ['options', 'assignments', 'preview'] as const
export type StrategyTab = (typeof STRATEGY_TABS)[number]

export interface StrategiesSearch extends TableSearch {
  /** Id of the strategy shown in the drawer. */
  strategy?: number | undefined
  /** Drawer tab other than the options. */
  tab?: Exclude<StrategyTab, 'options'> | undefined
}

export function parseStrategiesSearch(raw: Record<string, unknown>): StrategiesSearch {
  const id = typeof raw.strategy === 'string' ? Number(raw.strategy) : raw.strategy
  const tab = STRATEGY_TABS.find((candidate) => candidate === raw.tab)
  return {
    ...parseTableSearch(raw, { sortable: STRATEGY_SORTABLE }),
    strategy: typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? id : undefined,
    tab: tab === undefined || tab === 'options' ? undefined : tab,
  }
}
