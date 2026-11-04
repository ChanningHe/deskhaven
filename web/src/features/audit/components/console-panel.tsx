import { keepPreviousData } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { ScrollText } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListConsoleAudit } from '@/api/generated/audit/audit'
import type { ConsoleAuditView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { actorKindLabel } from '../audit-labels'
import { ACTOR_KINDS, auditListParams } from '../audit-search'
import { SelectFilter, TextFilter, UserFilter, WindowFilter } from './audit-filters'
import { AuditPanel } from './audit-panel'
import type { PanelProps } from './audit-panel'
import { Field, Fields, Optional, Time } from './detail-fields'
import { OpenButton } from './open-button'
import { SnapshotView } from './snapshot-view'

const column = createDataTableColumnHelper<ConsoleAuditView>()

/** The actor as a name when it still exists, else its kind (and id). */
function actorText(t: TFunction, actor: ConsoleAuditView['actor']): string {
  if (actor.name != null) return actor.name
  const kind = actorKindLabel(t, actor.kind)
  return actor.id == null ? kind : `${kind} #${String(actor.id)}`
}

function consoleColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
  return column.columns([
    column.accessor('created_at', {
      header: t('audit.columns.time'),
      enableHiding: false,
      cell: ({ row }) => (
        <OpenButton
          onOpen={() => {
            onOpen(row.original.id)
          }}
        >
          {formatDateTime(row.original.created_at, language)}
        </OpenButton>
      ),
    }),
    column.display({
      id: 'actor',
      header: t('audit.columns.actor'),
      meta: { label: t('audit.columns.actor') },
      cell: ({ row }) => actorText(t, row.original.actor),
    }),
    column.accessor('action', {
      header: t('audit.columns.action'),
      meta: { label: t('audit.columns.action'), className: 'font-mono text-xs' },
    }),
    column.display({
      id: 'target',
      header: t('audit.columns.target'),
      meta: { label: t('audit.columns.target'), className: 'max-w-56 truncate font-mono text-xs' },
      cell: ({ row }) => [row.original.target_type, row.original.target_id].join(' ').trim(),
    }),
    column.accessor('ip', {
      header: t('audit.columns.ip'),
      meta: { label: t('audit.columns.ip'), className: 'font-mono text-xs' },
      cell: ({ getValue }) => getValue() ?? '',
    }),
  ])
}

/** Changes made through the console, the admin API or the CLI, and the retention runs. */
export function ConsolePanel({ search, onSearchChange }: PanelProps) {
  const { t, i18n } = useTranslation()
  const query = useListConsoleAudit(auditListParams(search), {
    query: { placeholderData: keepPreviousData },
  })
  const columns = useMemo(
    () =>
      consoleColumns(t, i18n.language, (id) => {
        onSearchChange({ item: id })
      }),
    [t, i18n.language, onSearchChange],
  )
  return (
    <AuditPanel
      id="audit-console"
      label={t('audit.tabs.console')}
      search={search}
      onSearchChange={onSearchChange}
      query={query}
      columns={columns}
      searchPlaceholder={t('audit.search.console')}
      filters={
        <>
          <WindowFilter search={search} onChange={onSearchChange} />
          <UserFilter search={search} onChange={onSearchChange} />
          <SelectFilter
            label={t('audit.filters.actorKind')}
            value={search.actor_kind}
            options={ACTOR_KINDS.map((value) => ({ value, label: actorKindLabel(t, value) }))}
            onChange={(actor_kind) => {
              onSearchChange({ actor_kind })
            }}
          />
          <TextFilter
            label={t('audit.filters.action')}
            value={search.action}
            onCommit={(action) => {
              onSearchChange({ action })
            }}
          />
          <TextFilter
            label={t('audit.filters.targetType')}
            value={search.target_type}
            onCommit={(target_type) => {
              onSearchChange({ target_type })
            }}
          />
        </>
      }
      empty={{
        icon: ScrollText,
        title: t('audit.empty.console.title'),
        description: t('audit.empty.console.description'),
      }}
      detailTitle={(item) => item.action}
      detail={(item) => <ConsoleDetail item={item} />}
    />
  )
}

function ConsoleDetail({ item }: { item: ConsoleAuditView }) {
  const { t } = useTranslation()
  return (
    <>
      <Fields>
        <Field name={t('audit.detail.when')}>
          <Time at={item.created_at} />
        </Field>
        <Field name={t('audit.columns.actor')}>{actorText(t, item.actor)}</Field>
        <Field name={t('audit.columns.action')}>{item.action}</Field>
        <Field name={t('audit.detail.targetType')}>
          <Optional value={item.target_type} />
        </Field>
        <Field name={t('audit.detail.targetId')}>
          <Optional value={item.target_id} />
        </Field>
        <Field name={t('audit.columns.ip')}>
          <Optional value={item.ip} />
        </Field>
      </Fields>
      <SnapshotView action={item.action} before={item.before} after={item.after} />
    </>
  )
}
