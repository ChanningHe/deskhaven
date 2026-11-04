import type { TFunction } from 'i18next'

import type { ApiTokenView, Me } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import type { DataTableColumnDef } from '@/components/common/data-table'
import { Badge } from '@/components/ui/badge'
import { formatDateTime } from '@/lib/format'

import { RevokeButton } from './revoke-button'

const column = createDataTableColumnHelper<ApiTokenView>()

const STATUS_VARIANT = {
  active: 'secondary',
  revoked: 'destructive',
  expired: 'outline',
} as const

function dateOrDash(at: number | null | undefined, language: string, empty: string): string {
  return at === null || at === undefined ? empty : formatDateTime(at, language)
}

/** Columns of the token table; `onRevoke` asks to revoke one. */
export function tokenColumns(
  t: TFunction,
  language: string,
  me: Pick<Me, 'role'>,
  onRevoke: (token: ApiTokenView) => void,
): DataTableColumnDef<ApiTokenView>[] {
  return column.columns([
    column.accessor('name', {
      header: t('tokens.columns.name'),
      enableHiding: false,
      meta: { className: 'font-medium' },
    }),
    column.accessor('prefix', {
      header: t('tokens.columns.prefix'),
      meta: { label: t('tokens.columns.prefix'), className: 'font-mono text-xs' },
    }),
    column.accessor('scopes', {
      header: t('tokens.columns.scopes'),
      meta: { label: t('tokens.columns.scopes') },
      cell: ({ getValue }) => (
        <div className="flex flex-wrap gap-1">
          {getValue().map((scope) => (
            <Badge key={scope} variant="outline" className="font-mono">
              {scope}
            </Badge>
          ))}
        </div>
      ),
    }),
    column.display({
      id: 'created_by',
      header: t('tokens.columns.createdBy'),
      meta: { label: t('tokens.columns.createdBy') },
      cell: ({ row }) => row.original.created_by?.username ?? t('tokens.system'),
    }),
    column.accessor('created_at', {
      header: t('tokens.columns.createdAt'),
      meta: { label: t('tokens.columns.createdAt') },
      cell: ({ getValue }) => formatDateTime(getValue(), language),
    }),
    column.accessor('expires_at', {
      header: t('tokens.columns.expires'),
      meta: { label: t('tokens.columns.expires') },
      cell: ({ getValue }) => dateOrDash(getValue(), language, t('tokens.neverExpires')),
    }),
    column.accessor('last_used_at', {
      header: t('tokens.columns.lastUsed'),
      meta: { label: t('tokens.columns.lastUsed') },
      cell: ({ getValue }) => dateOrDash(getValue(), language, t('tokens.neverUsed')),
    }),
    column.accessor('status', {
      header: t('tokens.columns.status'),
      meta: { label: t('tokens.columns.status') },
      cell: ({ getValue }) => (
        <Badge variant={STATUS_VARIANT[getValue()]}>{t(`tokens.statuses.${getValue()}`)}</Badge>
      ),
    }),
    column.display({
      id: 'actions',
      header: () => <span className="sr-only">{t('tokens.columns.actions')}</span>,
      enableHiding: false,
      meta: { className: 'w-px text-right' },
      cell: ({ row }) => <RevokeButton me={me} token={row.original} onRevoke={onRevoke} />,
    }),
  ])
}
