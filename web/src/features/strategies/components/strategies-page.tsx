import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { SlidersHorizontal } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListStrategies } from '@/api/generated/strategies/strategies'
import { applyTableSearch, DataTable, toListParams } from '@/components/common/data-table'
import type { SearchChangeOptions, TableSearch } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

import { STRATEGY_SORTABLE } from '../strategies-search'
import type { StrategiesSearch } from '../strategies-search'
import { CreateStrategyButton } from './create-strategy-dialog'
import { strategyColumns } from './strategy-columns'
import { StrategyDrawer } from './strategy-drawer'

/** Strategies: the list, "new strategy", and a drawer per strategy. */
export function StrategiesPage() {
  const { t, i18n } = useTranslation()
  const search = useSearch({ from: '/_app/strategies' })
  const navigate = useNavigate({ from: '/strategies' })
  const query = useListStrategies(toListParams(search), {
    query: { placeholderData: keepPreviousData },
  })

  const onSearchChange = useCallback(
    (patch: TableSearch, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: StrategiesSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  // Opening the drawer adds a history entry; closing replaces it (Back never reopens it).
  const open = useCallback(
    (strategy: number) => {
      void navigate({
        search: (previous: StrategiesSearch) => ({ ...previous, strategy, tab: undefined }),
      })
    },
    [navigate],
  )
  const close = useCallback(() => {
    void navigate({
      search: (previous: StrategiesSearch) => ({
        ...previous,
        strategy: undefined,
        tab: undefined,
      }),
      replace: true,
    })
  }, [navigate])
  const columns = useMemo(() => strategyColumns(t, i18n.language, open), [t, i18n.language, open])

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('nav.strategies')}
        description={t('pages.strategies.description')}
        actions={<CreateStrategyButton onCreated={open} />}
      />
      <DataTable
        id="strategies"
        label={t('nav.strategies')}
        columns={columns}
        sortable={STRATEGY_SORTABLE}
        data={query.data}
        getRowId={(strategy) => String(strategy.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('strategies.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        emptyState={
          <EmptyState
            icon={SlidersHorizontal}
            title={t('pages.strategies.emptyTitle')}
            description={t('pages.strategies.emptyDescription')}
          />
        }
      />
      <StrategyDrawer strategyId={search.strategy} onClose={close} />
    </div>
  )
}
