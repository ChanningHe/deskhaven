import type { TFunction } from 'i18next'

import type { UserView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import type { DataTableColumnDef } from '@/components/common/data-table'
import { Badge } from '@/components/ui/badge'
import { formatDateTime } from '@/lib/format'

const column = createDataTableColumnHelper<UserView>()

const STATUS_VARIANT = {
  active: 'secondary',
  disabled: 'destructive',
  unverified: 'outline',
} as const

/** Columns of the user table; `onOpen` shows an account in the drawer. */
export function userColumns(
  t: TFunction,
  language: string,
  onOpen: (id: number) => void,
): DataTableColumnDef<UserView>[] {
  return column.columns([
    column.accessor('username', {
      header: t('users.columns.username'),
      enableHiding: false,
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
          onClick={() => {
            onOpen(row.original.id)
          }}
        >
          {row.original.username}
        </button>
      ),
    }),
    column.accessor('display_name', {
      header: t('users.columns.displayName'),
      meta: { label: t('users.columns.displayName') },
    }),
    column.accessor('role', {
      header: t('users.columns.role'),
      meta: { label: t('users.columns.role') },
      cell: ({ getValue }) => <Badge variant="outline">{t(`roles.${getValue()}`)}</Badge>,
    }),
    column.accessor('auth_source', {
      header: t('users.columns.source'),
      meta: { label: t('users.columns.source') },
      cell: ({ getValue }) => t(`users.sources.${getValue()}`),
    }),
    column.accessor('totp_enabled', {
      header: t('users.columns.tfa'),
      meta: { label: t('users.columns.tfa') },
      cell: ({ getValue }) => (getValue() ? t('users.tfaOn') : t('users.tfaOff')),
    }),
    column.accessor('status', {
      header: t('users.columns.status'),
      meta: { label: t('users.columns.status') },
      cell: ({ getValue }) => (
        <Badge variant={STATUS_VARIANT[getValue()]}>{t(`users.statuses.${getValue()}`)}</Badge>
      ),
    }),
    column.accessor('last_login_at', {
      header: t('users.columns.lastLogin'),
      meta: { label: t('users.columns.lastLogin') },
      cell: ({ getValue }) => {
        const at = getValue()
        return at === null || at === undefined ? t('users.never') : formatDateTime(at, language)
      },
    }),
  ])
}
