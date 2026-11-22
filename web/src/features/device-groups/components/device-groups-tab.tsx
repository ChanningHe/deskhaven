import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Boxes } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import { useListDeviceGroups } from '@/api/generated/device-groups/device-groups'
import type { DeviceGroupView } from '@/api/generated/model'
import {
  applyTableSearch,
  createDataTableColumnHelper,
  DataTable,
  toListParams,
} from '@/components/common/data-table'
import type { SearchChangeOptions, TableSearch } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { SourceBadge } from '@/components/common/source-badge'
import { GROUP_SORTABLE } from '@/features/groups/groups-search'
import type { GroupsSearch } from '@/features/groups/groups-search'

import { CreateDeviceGroupButton } from './create-device-group-dialog'
import { DeviceGroupDrawer } from './device-group-drawer'

const column = createDataTableColumnHelper<DeviceGroupView>()

function groupColumns(t: TFunction, onOpen: (id: number) => void) {
  return column.columns([
    column.accessor('name', {
      header: t('deviceGroups.columns.name'),
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
    column.accessor('device_count', {
      header: t('deviceGroups.columns.devices'),
      meta: { label: t('deviceGroups.columns.devices') },
    }),
    column.accessor('managed_by', {
      header: t('deviceGroups.columns.source'),
      meta: { label: t('deviceGroups.columns.source') },
      cell: ({ getValue }) =>
        getValue() === 'config' ? <SourceBadge source={{ kind: 'file' }} /> : null,
    }),
    column.accessor('note', {
      header: t('deviceGroups.columns.note'),
      meta: { label: t('deviceGroups.columns.note'), className: 'max-w-xs truncate' },
    }),
  ])
}

/** The device groups tab: list, create, and a drawer per group. */
export function DeviceGroupsTab() {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/groups' })
  const navigate = useNavigate({ from: '/groups' })
  const query = useListDeviceGroups(toListParams(search), {
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
      <div className="flex items-start justify-between gap-3">
        <p className="text-sm text-muted-foreground">{t('deviceGroups.description')}</p>
        <CreateDeviceGroupButton onCreated={openGroup} />
      </div>
      <DataTable
        id="device-groups"
        label={t('deviceGroups.tableLabel')}
        columns={columns}
        sortable={GROUP_SORTABLE}
        data={query.data}
        getRowId={(group) => String(group.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('deviceGroups.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        emptyState={
          <EmptyState
            icon={Boxes}
            title={t('deviceGroups.emptyTitle')}
            description={t('deviceGroups.emptyDescription')}
          />
        }
      />
      <DeviceGroupDrawer groupId={search.group} onClose={closeGroup} />
    </div>
  )
}
