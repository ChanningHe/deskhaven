import type { RowData, Table } from '@tanstack/react-table'
import { FlexRender } from '@tanstack/react-table'
import { LoaderCircle } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { TableBody, TableCell, TableRow } from '@/components/ui/table'

import type { DataTableFeatures } from './columns'

export type BodyState =
  | { kind: 'pending' }
  | { kind: 'error'; error: unknown; onRetry: (() => void) | undefined }
  | { kind: 'no-results'; onClearFilters: (() => void) | undefined }
  | { kind: 'rows' }

function MessageRow({ span, children }: { span: number; children: ReactNode }) {
  return (
    <TableRow className="hover:bg-transparent">
      <TableCell colSpan={span} className="py-10 text-center text-muted-foreground">
        {children}
      </TableCell>
    </TableRow>
  )
}

function StateRow({ state, span }: { state: Exclude<BodyState, { kind: 'rows' }>; span: number }) {
  const { t } = useTranslation()
  switch (state.kind) {
    case 'pending':
      return (
        <MessageRow span={span}>
          <span role="status" className="inline-flex items-center gap-2">
            <LoaderCircle aria-hidden className="size-4 animate-spin" />
            {t('common.loading')}
          </span>
        </MessageRow>
      )
    case 'error':
      return (
        <MessageRow span={span}>
          <ErrorAlert
            error={state.error}
            title={t('dataTable.loadFailed')}
            className="text-left"
            {...(state.onRetry ? { onRetry: state.onRetry } : {})}
          />
        </MessageRow>
      )
    case 'no-results':
      return (
        <MessageRow span={span}>
          <p>{t('dataTable.noResults')}</p>
          {state.onClearFilters ? (
            <Button variant="link" onClick={state.onClearFilters}>
              {t('dataTable.clearFilters')}
            </Button>
          ) : null}
        </MessageRow>
      )
  }
}

export function DataTableBody<TData extends RowData>({
  table,
  state,
}: {
  table: Table<DataTableFeatures, TData>
  state: BodyState
}) {
  const span = Math.max(1, table.getVisibleLeafColumns().length)
  if (state.kind !== 'rows') {
    return (
      <TableBody>
        <StateRow state={state} span={span} />
      </TableBody>
    )
  }
  return (
    <TableBody>
      {table.getRowModel().rows.map((row) => (
        <TableRow key={row.id} data-state={row.getIsSelected() ? 'selected' : undefined}>
          {row.getVisibleCells().map((cell) => (
            <TableCell key={cell.id} className={cell.column.columnDef.meta?.className}>
              <FlexRender cell={cell} />
            </TableCell>
          ))}
        </TableRow>
      ))}
    </TableBody>
  )
}
