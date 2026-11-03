import type { TFunction } from 'i18next'

import type { DeviceView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import type { DataTableColumnDef } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { DeviceStatusBadge } from './device-status-badge'

const column = createDataTableColumnHelper<DeviceView>()

/** Columns of the device table; `onOpen` shows a device in the drawer. */
export function deviceColumns(
  t: TFunction,
  language: string,
  onOpen: (id: number) => void,
): DataTableColumnDef<DeviceView>[] {
  return column.columns([
    column.display({
      id: 'status',
      header: t('devices.columns.status'),
      meta: { label: t('devices.columns.status') },
      cell: ({ row }) => <DeviceStatusBadge device={row.original} />,
    }),
    column.accessor('rustdesk_id', {
      header: t('devices.columns.id'),
      enableHiding: false,
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-mono font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
          onClick={() => {
            onOpen(row.original.id)
          }}
        >
          {row.original.rustdesk_id}
        </button>
      ),
    }),
    column.accessor('hostname', {
      header: t('devices.columns.hostname'),
      meta: { label: t('devices.columns.hostname') },
      cell: ({ getValue }) => getValue() ?? '',
    }),
    column.display({
      id: 'owner',
      header: t('devices.columns.owner'),
      meta: { label: t('devices.columns.owner') },
      cell: ({ row }) => row.original.owner?.username ?? '',
    }),
    column.display({
      id: 'device_group',
      header: t('devices.columns.group'),
      meta: { label: t('devices.columns.group') },
      cell: ({ row }) => row.original.device_group?.name ?? '',
    }),
    column.accessor('os', {
      header: t('devices.columns.os'),
      meta: { label: t('devices.columns.os'), className: 'max-w-48 truncate' },
      cell: ({ getValue }) => getValue() ?? '',
    }),
    column.accessor('version', {
      header: t('devices.columns.version'),
      meta: { label: t('devices.columns.version') },
      cell: ({ getValue }) => getValue() ?? '',
    }),
    column.accessor('last_seen_at', {
      header: t('devices.columns.lastSeen'),
      meta: { label: t('devices.columns.lastSeen') },
      cell: ({ getValue }) => formatDateTime(getValue(), language),
    }),
  ])
}
