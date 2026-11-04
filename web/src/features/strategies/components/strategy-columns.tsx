import type { TFunction } from 'i18next'

import type { StrategyView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { SourceBadge } from '@/components/common/source-badge'
import { formatDateTime } from '@/lib/format'

const column = createDataTableColumnHelper<StrategyView>()

/** Options the strategy sets, secret ones included. */
export function optionCount(strategy: StrategyView): number {
  return Object.keys(strategy.options).length + strategy.secret_options.length
}

export function strategyColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
  return column.columns([
    column.accessor('name', {
      header: t('strategies.columns.name'),
      enableHiding: false,
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
          onClick={() => {
            onOpen(row.original.id)
          }}
        >
          {row.original.name}
        </button>
      ),
    }),
    column.display({
      id: 'options',
      header: t('strategies.columns.options'),
      meta: { label: t('strategies.columns.options') },
      cell: ({ row }) => optionCount(row.original),
    }),
    column.accessor('assignment_count', {
      header: t('strategies.columns.assignments'),
      meta: { label: t('strategies.columns.assignments') },
    }),
    column.accessor('managed_by', {
      header: t('strategies.columns.source'),
      meta: { label: t('strategies.columns.source') },
      cell: ({ getValue }) =>
        getValue() === 'config' ? <SourceBadge source={{ kind: 'file' }} /> : null,
    }),
    column.accessor('updated_at', {
      header: t('strategies.columns.updated'),
      meta: { label: t('strategies.columns.updated') },
      cell: ({ getValue }) => formatDateTime(getValue(), language),
    }),
    column.accessor('note', {
      header: t('strategies.columns.note'),
      meta: { label: t('strategies.columns.note'), className: 'max-w-xs truncate' },
    }),
  ])
}
