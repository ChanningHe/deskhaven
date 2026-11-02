import type { RowData, Table } from '@tanstack/react-table'
import { Columns3, Search } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { ReactNode, SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'

import type { DataTableFeatures } from './columns'

export const SEARCH_DEBOUNCE_MS = 300

interface SearchBoxProps {
  value: string | undefined
  placeholder: string
  onChange: (q: string) => void
}

/** Free-text search (`q`), applied after a short pause or on Enter. */
function SearchBox({ value = '', placeholder, onChange }: SearchBoxProps) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(value)
  const [synced, setSynced] = useState(value)
  // The URL changed underneath (back button, "clear filters"): follow it.
  if (value !== synced) {
    setSynced(value)
    setDraft(value)
  }

  useEffect(() => {
    if (draft.trim() === value) return
    const timer = setTimeout(() => {
      onChange(draft.trim())
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [draft, value, onChange])

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    onChange(draft.trim())
  }

  return (
    <form role="search" onSubmit={submit} className="relative w-full max-w-xs">
      <Search
        aria-hidden
        className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
      />
      <Input
        type="search"
        aria-label={t('dataTable.search')}
        placeholder={placeholder}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value)
        }}
        className="pl-8"
      />
    </form>
  )
}

function ColumnPicker<TData extends RowData>({
  table,
}: {
  table: Table<DataTableFeatures, TData>
}) {
  const { t } = useTranslation()
  const columns = table.getAllLeafColumns().filter((column) => column.getCanHide())
  if (columns.length === 0) return null

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="outline" />}>
        <Columns3 aria-hidden />
        {t('dataTable.columns')}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t('dataTable.toggleColumns')}</DropdownMenuLabel>
          {columns.map((column) => (
            <DropdownMenuCheckboxItem
              key={column.id}
              checked={column.getIsVisible()}
              onCheckedChange={(checked) => {
                column.toggleVisibility(checked)
              }}
            >
              {column.columnDef.meta?.label ?? column.id}
            </DropdownMenuCheckboxItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

interface DataTableToolbarProps<TData extends RowData> {
  table: Table<DataTableFeatures, TData>
  q: string | undefined
  searchPlaceholder: string | undefined
  onSearch: (q: string) => void
  filters: ReactNode
}

export function DataTableToolbar<TData extends RowData>({
  table,
  q,
  searchPlaceholder,
  onSearch,
  filters,
}: DataTableToolbarProps<TData>) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      {searchPlaceholder === undefined ? null : (
        <SearchBox value={q} placeholder={searchPlaceholder} onChange={onSearch} />
      )}
      {filters}
      <div className="ml-auto">
        <ColumnPicker table={table} />
      </div>
    </div>
  )
}
