import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { BookUser } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import { useListAddressBooks } from '@/api/generated/address-books/address-books'
import type { BookView } from '@/api/generated/model'
import {
  applyTableSearch,
  createDataTableColumnHelper,
  DataTable,
  toListParams,
} from '@/components/common/data-table'
import type { SearchChangeOptions, TableSearch } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { formatDateTime } from '@/lib/format'

import { BOOK_SORTABLE } from '../addressbooks-search'
import type { AddressBooksSearch } from '../addressbooks-search'
import { BookDrawer } from './book-drawer'
import { CreateBookButton } from './create-book-dialog'

const column = createDataTableColumnHelper<BookView>()

function bookColumns(t: TFunction, language: string, onOpen: (guid: string) => void) {
  return column.columns([
    column.accessor('name', {
      header: t('addressBooks.columns.name'),
      enableHiding: false,
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
          onClick={() => {
            onOpen(row.original.guid)
          }}
        >
          {row.original.name}
        </button>
      ),
    }),
    column.accessor('owner', {
      header: t('addressBooks.columns.owner'),
      meta: { label: t('addressBooks.columns.owner') },
      cell: ({ getValue }) => getValue() ?? t('addressBooks.ownerNone'),
    }),
    column.accessor('peer_count', {
      header: t('addressBooks.columns.entries'),
      meta: { label: t('addressBooks.columns.entries') },
    }),
    column.accessor('tag_count', {
      header: t('addressBooks.columns.tags'),
      meta: { label: t('addressBooks.columns.tags') },
    }),
    column.accessor('share_count', {
      header: t('addressBooks.columns.shares'),
      meta: { label: t('addressBooks.columns.shares') },
    }),
    column.accessor('updated_at', {
      header: t('addressBooks.columns.updated'),
      meta: { label: t('addressBooks.columns.updated') },
      cell: ({ getValue }) => formatDateTime(getValue(), language),
    }),
  ])
}

/** Shared address books: list, create, and a drawer per book (entries, tags, grants). */
export function AddressBooksPage() {
  const { t, i18n } = useTranslation()
  const search = useSearch({ from: '/_app/address-books' })
  const navigate = useNavigate({ from: '/address-books' })
  const query = useListAddressBooks(toListParams(search), {
    query: { placeholderData: keepPreviousData },
  })

  const onSearchChange = useCallback(
    (patch: TableSearch, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: AddressBooksSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  // Opening the drawer adds a history entry; closing replaces it (Back never reopens it).
  const openBook = useCallback(
    (book: string) => {
      void navigate({ search: (previous: AddressBooksSearch) => ({ ...previous, book }) })
    },
    [navigate],
  )
  const closeBook = useCallback(() => {
    void navigate({
      search: (previous: AddressBooksSearch) => ({
        ...previous,
        book: undefined,
        tab: undefined,
      }),
      replace: true,
    })
  }, [navigate])
  const columns = useMemo(
    () => bookColumns(t, i18n.language, openBook),
    [t, i18n.language, openBook],
  )

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('nav.addressBooks')}
        description={t('pages.addressBooks.description')}
        actions={<CreateBookButton onCreated={openBook} />}
      />
      <DataTable
        id="address-books"
        label={t('addressBooks.tableLabel')}
        columns={columns}
        sortable={BOOK_SORTABLE}
        data={query.data}
        getRowId={(book) => book.guid}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('addressBooks.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        emptyState={
          <EmptyState
            icon={BookUser}
            title={t('pages.addressBooks.emptyTitle')}
            description={t('pages.addressBooks.emptyDescription')}
          />
        }
      />
      <BookDrawer guid={search.book} onClose={closeBook} />
    </div>
  )
}
