import { useCallback } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { LucideIcon } from 'lucide-react'

import { DataTable } from '@/components/common/data-table'
import type {
  DataTableColumnDef,
  ListPage,
  SearchChangeOptions,
} from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'

import { clearedFilters, hasAuditFilters } from '../audit-search'
import type { AuditSearch } from '../audit-search'

/** What every tab's panel gets from the page. */
export interface PanelProps {
  search: AuditSearch
  /** Applies a change to the route search (see `applyTableSearch`). */
  onSearchChange: (patch: Partial<AuditSearch>, options?: SearchChangeOptions) => void
}

interface AuditPanelProps<T extends { id: number }> extends PanelProps {
  /** Table id (keys the persisted column visibility) and accessible name. */
  id: string
  label: string
  query: { data: ListPage<T> | undefined; error: unknown; refetch: () => Promise<unknown> }
  columns: readonly DataTableColumnDef<T>[]
  filters: ReactNode
  /** Placeholder of the free-text search; absent: the list has none. */
  searchPlaceholder?: string
  empty: { icon: LucideIcon; title: string; description: string }
  /** Title of the drawer for the open record. */
  detailTitle: (item: T) => string
  detail: (item: T) => ReactNode
}

/**
 * One audit list: the table with its filters and, for the record named by `item` in the
 * URL, a read-only drawer. A record that is not on the current page shows no drawer.
 */
export function AuditPanel<T extends { id: number }>(props: AuditPanelProps<T>) {
  const { t } = useTranslation()
  const { search, onSearchChange, query, detailTitle, detail } = props
  const selected = query.data?.items.find((row) => row.id === search.item)
  const closeDrawer = useCallback(() => {
    onSearchChange({ item: undefined }, { replace: true })
  }, [onSearchChange])

  return (
    <>
      <DataTable
        id={props.id}
        label={props.label}
        columns={props.columns}
        sortable={['created_at']}
        data={query.data}
        getRowId={(row) => String(row.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        {...(props.searchPlaceholder === undefined
          ? {}
          : { searchPlaceholder: props.searchPlaceholder })}
        filters={props.filters}
        isFiltered={hasAuditFilters(search)}
        onClearFilters={() => {
          onSearchChange(clearedFilters(search))
        }}
        emptyState={
          <EmptyState
            icon={props.empty.icon}
            title={props.empty.title}
            description={props.empty.description}
          />
        }
      />
      <Sheet
        open={selected !== undefined}
        onOpenChange={(open) => {
          if (!open) closeDrawer()
        }}
      >
        <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-lg">
          {selected === undefined ? null : (
            <>
              <SheetHeader>
                <SheetTitle>{detailTitle(selected)}</SheetTitle>
                <SheetDescription>{t('audit.detail.hint')}</SheetDescription>
              </SheetHeader>
              <div className="grid gap-4 px-4 pb-4">{detail(selected)}</div>
            </>
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
