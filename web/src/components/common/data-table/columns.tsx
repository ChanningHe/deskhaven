import {
  columnVisibilityFeature,
  createColumnHelper,
  rowSelectionFeature,
  tableFeatures,
} from '@tanstack/react-table'
import type { ColumnDef, RowData } from '@tanstack/react-table'

import { SelectAllCheckbox, SelectRowCheckbox } from './selection-checkboxes'

export interface DataTableColumnMeta {
  /** Required for a hideable column whose header is not a string. */
  label?: string
  className?: string
}

// Type-only slot: tells Table v9 the shape of `columnDef.meta`.
const COLUMN_META: DataTableColumnMeta = {}

/** Sorting, filtering and pagination are server-side, owned by the URL. */
export const dataTableFeatures = tableFeatures({
  rowSelectionFeature,
  columnVisibilityFeature,
  columnMeta: COLUMN_META,
})

export type DataTableFeatures = typeof dataTableFeatures

export type DataTableColumnDef<TData extends RowData> = ColumnDef<DataTableFeatures, TData>

/** Build the list with `helper.columns([...])` at module scope so the array is stable. */
export function createDataTableColumnHelper<TData extends RowData>() {
  return createColumnHelper<DataTableFeatures, TData>()
}

export const SELECT_COLUMN_ID = '_select'

export function selectColumn<TData extends RowData>(
  getRowLabel?: (row: TData) => string,
): DataTableColumnDef<TData> {
  return {
    id: SELECT_COLUMN_ID,
    enableHiding: false,
    meta: { className: 'w-10' },
    header: ({ table }) => (
      <SelectAllCheckbox
        checked={table.getIsAllPageRowsSelected()}
        indeterminate={table.getIsSomePageRowsSelected()}
        onChange={(checked) => {
          table.toggleAllPageRowsSelected(checked)
        }}
      />
    ),
    cell: ({ row }) => (
      <SelectRowCheckbox
        name={getRowLabel?.(row.original)}
        checked={row.getIsSelected()}
        onChange={(checked) => {
          row.toggleSelected(checked)
        }}
      />
    ),
  }
}
