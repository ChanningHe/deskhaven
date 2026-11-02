import { useTable } from '@tanstack/react-table'
import type { RowData, RowSelectionState, Updater } from '@tanstack/react-table'
import { useCallback, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'

import { Table, TableHeader, TableRow } from '@/components/ui/table'

import { BulkActionBar } from './bulk-action-bar'
import { dataTableFeatures, selectColumn } from './columns'
import type { DataTableColumnDef } from './columns'
import { DataTableBody } from './data-table-body'
import type { BodyState } from './data-table-body'
import { DataTableHead } from './data-table-head'
import { DataTablePagination } from './data-table-pagination'
import { DataTableToolbar } from './data-table-toolbar'
import { formatSort, nextSort, pageCount, parseSort, toListParams } from './table-search'
import type { ListPage, TableSearch } from './table-search'
import { useColumnVisibility } from './use-column-visibility'

export interface DataTableProps<TData extends RowData> {
  /** Stable id: keys the persisted column visibility. */
  id: string
  /** Accessible name of the table, usually the page title. */
  label: string
  /** Column definitions; keep the array stable (module scope or useMemo). */
  columns: readonly DataTableColumnDef<TData>[]
  /** Column ids the server can sort by (the same list the route validates). */
  sortable?: readonly string[]
  /** The current page from the list endpoint; undefined while loading. */
  data: ListPage<TData> | undefined
  getRowId: (row: TData) => string
  /** The table's part of the route search (page, page_size, sort, q). */
  search: TableSearch
  /** Applies a change to the route search; pages call `applyTableSearch`. */
  onSearchChange: (patch: TableSearch) => void
  error?: unknown
  onRetry?: () => void
  /** Shows the free-text search box (`q`) with this placeholder. */
  searchPlaceholder?: string
  /** Page-specific filter controls, rendered next to the search box. */
  filters?: ReactNode
  /** Whether page-specific filters are active (`q` is detected automatically). */
  isFiltered?: boolean
  onClearFilters?: () => void
  /** Enables row selection; renders the actions for the selected row ids. */
  bulkActions?: (selectedIds: string[], clearSelection: () => void) => ReactNode
  /** Shown instead of the table when the list is empty without any filter. */
  emptyState: ReactNode
}

const NO_ROWS: never[] = []

function bodyState<TData extends RowData>(
  props: DataTableProps<TData>,
  isFiltered: boolean,
): BodyState {
  const { data, error, onRetry, onClearFilters } = props
  if (data === undefined) {
    return error === undefined ? { kind: 'pending' } : { kind: 'error', error, onRetry }
  }
  if (data.items.length === 0 && isFiltered) return { kind: 'no-results', onClearFilters }
  return { kind: 'rows' }
}

export function DataTable<TData extends RowData>(props: DataTableProps<TData>) {
  const { id, label, columns, sortable = [], data, getRowId, search, onSearchChange } = props
  const { bulkActions, emptyState, searchPlaceholder, filters } = props
  const isFiltered = (props.isFiltered ?? false) || search.q !== undefined
  const params = toListParams(search)
  const sort = parseSort(search.sort)

  const allColumns = useMemo(
    () => (bulkActions ? [selectColumn<TData>(), ...columns] : columns),
    [bulkActions, columns],
  )
  const [columnVisibility, setColumnVisibility] = useColumnVisibility(id)

  // Selection belongs to the rows on screen: a new page, sort or filter clears it.
  const selectionKey = JSON.stringify(params)
  const [selection, setSelection] = useState<{ key: string; rows: RowSelectionState }>({
    key: selectionKey,
    rows: {},
  })
  const rowSelection = selection.key === selectionKey ? selection.rows : {}
  const onRowSelectionChange = (updater: Updater<RowSelectionState>) => {
    const rows = typeof updater === 'function' ? updater(rowSelection) : updater
    setSelection({ key: selectionKey, rows })
  }

  const table = useTable({
    features: dataTableFeatures,
    columns: allColumns,
    data: data?.items ?? NO_ROWS,
    getRowId,
    state: { columnVisibility, rowSelection },
    onColumnVisibilityChange: setColumnVisibility,
    onRowSelectionChange,
    enableRowSelection: bulkActions !== undefined,
  })

  // A page past the end (rows deleted elsewhere, a stale link): go to the last one.
  const pages = data === undefined ? undefined : pageCount(data.total, params.page_size)
  useEffect(() => {
    if (pages !== undefined && params.page > pages) onSearchChange({ page: pages })
  }, [pages, params.page, onSearchChange])

  const onSearch = useCallback(
    (q: string) => {
      onSearchChange({ q })
    },
    [onSearchChange],
  )

  if (data?.total === 0 && !isFiltered) return <>{emptyState}</>

  const selectedIds = Object.keys(rowSelection).filter((rowId) => rowSelection[rowId])
  const clearSelection = () => {
    setSelection({ key: selectionKey, rows: {} })
  }

  return (
    <div className="space-y-3">
      <DataTableToolbar
        table={table}
        q={search.q}
        searchPlaceholder={searchPlaceholder}
        onSearch={onSearch}
        filters={filters}
      />
      {bulkActions && selectedIds.length > 0 ? (
        <BulkActionBar count={selectedIds.length} onClear={clearSelection}>
          {bulkActions(selectedIds, clearSelection)}
        </BulkActionBar>
      ) : null}
      <div className="rounded-lg border" aria-busy={data === undefined}>
        <Table aria-label={label}>
          <TableHeader>
            {table.getHeaderGroups().map((group) => (
              <TableRow key={group.id} className="hover:bg-transparent">
                {group.headers.map((header) => (
                  <DataTableHead
                    key={header.id}
                    header={header}
                    sortable={sortable.includes(header.column.id)}
                    sort={sort}
                    onSort={(columnId) => {
                      onSearchChange({ sort: formatSort(nextSort(sort, columnId)) })
                    }}
                  />
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <DataTableBody table={table} state={bodyState(props, isFiltered)} />
        </Table>
      </div>
      {data === undefined || data.total === 0 ? null : (
        <DataTablePagination
          page={params.page}
          pageSize={params.page_size}
          total={data.total}
          onPageChange={(page) => {
            onSearchChange({ page })
          }}
          onPageSizeChange={(pageSize) => {
            onSearchChange({ page_size: pageSize })
          }}
        />
      )}
    </div>
  )
}
