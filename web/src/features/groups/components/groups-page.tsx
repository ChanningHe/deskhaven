import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Boxes } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import { useListGroups } from '@/api/generated/groups/groups'
import type { GroupView } from '@/api/generated/model'
import {
  applyTableSearch,
  createDataTableColumnHelper,
  DataTable,
  toListParams,
} from '@/components/common/data-table'
import type { SearchChangeOptions, TableSearch } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { SourceBadge } from '@/components/common/source-badge'

import { GROUP_SORTABLE } from '../groups-search'
import type { GroupsSearch } from '../groups-search'
import { CreateGroupButton } from './create-group-dialog'
import { GroupDrawer } from './group-drawer'

const column = createDataTableColumnHelper<GroupView>()

function groupColumns(t: TFunction, onOpen: (id: number) => void) {
  return column.columns([
    column.accessor('name', {
      header: t('groups.columns.name'),
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
    column.accessor('member_count', {
      header: t('groups.columns.members'),
      meta: { label: t('groups.columns.members') },
    }),
    column.accessor('managed_by', {
      header: t('groups.columns.source'),
      meta: { label: t('groups.columns.source') },
      cell: ({ getValue }) =>
        getValue() === 'config' ? <SourceBadge source={{ kind: 'file' }} /> : null,
    }),
    column.accessor('note', {
      header: t('groups.columns.note'),
      meta: { label: t('groups.columns.note'), className: 'max-w-xs truncate' },
    }),
  ])
}

/** User groups: list, create, and a drawer per group with its members. */
export function GroupsPage() {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/groups' })
  const navigate = useNavigate({ from: '/groups' })
  const query = useListGroups(toListParams(search), {
    query: { placeholderData: keepPreviousData },
  })

  const onSearchChange = useCallback(
    (patch: TableSearch, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: GroupsSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  // Opening the drawer adds a history entry; closing replaces it (Back never reopens it).
  const openGroup = useCallback(
    (group: number) => {
      void navigate({ search: (previous: GroupsSearch) => ({ ...previous, group }) })
    },
    [navigate],
  )
  const closeGroup = useCallback(() => {
    void navigate({
      search: (previous: GroupsSearch) => ({ ...previous, group: undefined }),
      replace: true,
    })
  }, [navigate])
  const columns = useMemo(() => groupColumns(t, openGroup), [t, openGroup])

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('nav.groups')}
        description={t('groups.description')}
        actions={<CreateGroupButton onCreated={openGroup} />}
      />
      <DataTable
        id="groups"
        label={t('groups.tableLabel')}
        columns={columns}
        sortable={GROUP_SORTABLE}
        data={query.data}
        getRowId={(group) => String(group.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('groups.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        emptyState={
          <EmptyState
            icon={Boxes}
            title={t('pages.groups.emptyTitle')}
            description={t('pages.groups.emptyDescription')}
          />
        }
      />
      <GroupDrawer groupId={search.group} onClose={closeGroup} />
    </div>
  )
}
