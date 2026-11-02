import type { Header, RowData } from '@tanstack/react-table'
import { FlexRender } from '@tanstack/react-table'
import { ArrowDown, ArrowUp, ChevronsUpDown } from 'lucide-react'

import { TableHead } from '@/components/ui/table'
import { cn } from '@/lib/utils'

import type { DataTableFeatures } from './columns'
import type { SortState } from './table-search'

interface DataTableHeadProps<TData extends RowData> {
  header: Header<DataTableFeatures, TData>
  sortable: boolean
  sort: SortState | undefined
  onSort: (columnId: string) => void
}

const ARIA_SORT = { asc: 'ascending', desc: 'descending' } as const

/**
 * A column header. Sortable columns render a button that cycles the sort and
 * expose the current direction through `aria-sort` on the header cell.
 */
export function DataTableHead<TData extends RowData>({
  header,
  sortable,
  sort,
  onSort,
}: DataTableHeadProps<TData>) {
  const { column } = header
  const meta = column.columnDef.meta
  const content = header.isPlaceholder ? null : <FlexRender header={header} />

  if (!sortable) {
    return <TableHead className={meta?.className}>{content}</TableHead>
  }

  const direction = sort?.id === column.id ? (sort.desc ? 'desc' : 'asc') : undefined
  const Icon = direction === 'asc' ? ArrowUp : direction === 'desc' ? ArrowDown : ChevronsUpDown

  return (
    <TableHead
      className={meta?.className}
      aria-sort={direction === undefined ? 'none' : ARIA_SORT[direction]}
    >
      <button
        type="button"
        className="-ml-2 inline-flex h-7 items-center gap-1 rounded-md px-2 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        onClick={() => {
          onSort(column.id)
        }}
      >
        {content}
        <Icon
          aria-hidden
          className={cn('size-3.5', direction === undefined && 'text-muted-foreground/60')}
        />
      </button>
    </TableHead>
  )
}
