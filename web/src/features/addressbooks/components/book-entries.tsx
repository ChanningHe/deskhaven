import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { Download, KeyRound, Monitor, Plus } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import {
  deleteAddressBookPeers,
  useListAddressBookPeers,
} from '@/api/generated/address-books/address-books'
import type { BookView, PeerView } from '@/api/generated/model'
import {
  applyTableSearch,
  createDataTableColumnHelper,
  DataTable,
  toListParams,
} from '@/components/common/data-table'
import type { TableSearch } from '@/components/common/data-table'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { EmptyState } from '@/components/common/empty-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'

import { invalidateAddressBooks } from '../invalidate'
import { ImportDevicesDialog } from './import-devices-dialog'
import { PeerFormDialog } from './peer-form-dialog'

const column = createDataTableColumnHelper<PeerView>()

function peerColumns(t: TFunction, onEdit: (peer: PeerView) => void) {
  return column.columns([
    column.accessor('id', {
      header: t('addressBooks.entries.columns.id'),
      enableHiding: false,
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
          onClick={() => {
            onEdit(row.original)
          }}
        >
          {row.original.id}
        </button>
      ),
    }),
    column.accessor('alias', {
      header: t('addressBooks.entries.columns.alias'),
      meta: { label: t('addressBooks.entries.columns.alias') },
    }),
    column.accessor('hostname', {
      header: t('addressBooks.entries.columns.hostname'),
      meta: { label: t('addressBooks.entries.columns.hostname') },
    }),
    column.accessor('username', {
      header: t('addressBooks.entries.columns.username'),
      meta: { label: t('addressBooks.entries.columns.username') },
    }),
    column.accessor('tags', {
      header: t('addressBooks.entries.columns.tags'),
      meta: { label: t('addressBooks.entries.columns.tags') },
      cell: ({ getValue }) => (
        <span className="flex flex-wrap gap-1">
          {getValue().map((tag) => (
            <Badge key={tag} variant="secondary">
              {tag}
            </Badge>
          ))}
        </span>
      ),
    }),
    column.accessor('has_password', {
      header: t('addressBooks.entries.columns.password'),
      meta: { label: t('addressBooks.entries.columns.password') },
      cell: ({ getValue }) =>
        getValue() ? (
          <Badge variant="outline">
            <KeyRound aria-hidden />
            {t('addressBooks.entries.hasPassword')}
          </Badge>
        ) : null,
    }),
  ])
}

/** The entries of one book: search, page, add, edit, import from devices, remove. */
export function BookEntries({ book }: { book: BookView }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  // The drawer owns the URL's `page` / `q`; the entries keep their own list state.
  const [search, setSearch] = useState<TableSearch>({})
  const [editing, setEditing] = useState<PeerView | 'new' | null>(null)
  const [importing, setImporting] = useState(false)
  const [removing, setRemoving] = useState<{ ids: string[]; clear: () => void } | null>(null)
  const [confirming, setConfirming] = useState(false)
  const { page, page_size, q } = toListParams(search)
  const query = useListAddressBookPeers(
    book.guid,
    { page, page_size, ...(q === undefined ? {} : { q }) },
    { query: { placeholderData: keepPreviousData } },
  )
  const onSearchChange = useCallback((patch: TableSearch) => {
    setSearch((previous) => applyTableSearch(previous, patch))
  }, [])
  const columns = useMemo(() => peerColumns(t, setEditing), [t])

  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap justify-end gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            setImporting(true)
          }}
        >
          <Download aria-hidden />
          {t('addressBooks.entries.import')}
        </Button>
        <Button
          size="sm"
          onClick={() => {
            setEditing('new')
          }}
        >
          <Plus aria-hidden />
          {t('addressBooks.entries.add')}
        </Button>
      </div>
      <DataTable
        id="address-book-entries"
        label={t('addressBooks.entries.tableLabel')}
        columns={columns}
        data={query.data}
        getRowId={(peer) => peer.id}
        getRowLabel={(peer) => peer.id}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('addressBooks.entries.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        bulkActions={(ids, clear) => (
          <Button
            size="sm"
            variant="destructive"
            onClick={() => {
              setRemoving({ ids, clear })
              setConfirming(true)
            }}
          >
            {t('addressBooks.entries.removeSelected')}
          </Button>
        )}
        emptyState={
          <EmptyState
            icon={Monitor}
            title={t('addressBooks.entries.emptyTitle')}
            description={t('addressBooks.entries.emptyDescription')}
          />
        }
      />
      <PeerFormDialog
        guid={book.guid}
        peer={editing}
        onClose={() => {
          setEditing(null)
        }}
      />
      <ImportDevicesDialog guid={book.guid} open={importing} onOpenChange={setImporting} />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('addressBooks.entries.confirmRemove.title')}
        description={t('addressBooks.entries.confirmRemove.description', {
          count: removing?.ids.length ?? 0,
        })}
        confirmLabel={t('addressBooks.entries.confirmRemove.confirm')}
        onConfirm={async () => {
          if (removing === null) return
          await deleteAddressBookPeers(book.guid, { ids: removing.ids })
          removing.clear()
          await invalidateAddressBooks(queryClient)
        }}
      />
    </div>
  )
}
