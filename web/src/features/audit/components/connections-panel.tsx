import { keepPreviousData } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { Cable } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListConnectionAudit } from '@/api/generated/audit/audit'
import type { ConnectionAuditView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { CONN_TYPE_CODES, connTypeLabel, primaryAuthLabel, twoFactorLabel } from '../audit-labels'
import { auditListParams } from '../audit-search'
import { SelectFilter, TextFilter, UserFilter, WindowFilter } from './audit-filters'
import { AuditPanel } from './audit-panel'
import type { PanelProps } from './audit-panel'
import { Field, Fields, Optional, SelfReported, Time } from './detail-fields'
import { OpenButton } from './open-button'

const column = createDataTableColumnHelper<ConnectionAuditView>()

function connectionColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
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
    column.accessor('device_id', {
      header: t('audit.columns.device'),
      meta: { label: t('audit.columns.device'), className: 'font-mono' },
    }),
    column.display({
      id: 'peer',
      header: t('audit.columns.peer'),
      meta: { label: t('audit.columns.peer'), className: 'max-w-48 truncate' },
      cell: ({ row }) => row.original.peer_name ?? row.original.peer_id ?? '',
    }),
    column.display({
      id: 'controller',
      header: t('audit.columns.controller'),
      meta: { label: t('audit.columns.controller') },
      cell: ({ row }) => row.original.controller?.username ?? '',
    }),
    column.display({
      id: 'conn_type',
      header: t('audit.columns.type'),
      meta: { label: t('audit.columns.type') },
      cell: ({ row }) =>
        row.original.conn_type == null ? '' : connTypeLabel(t, row.original.conn_type),
    }),
    column.accessor('ip', {
      header: t('audit.columns.ip'),
      meta: { label: t('audit.columns.ip'), className: 'font-mono text-xs' },
      cell: ({ getValue }) => getValue() ?? '',
    }),
    column.display({
      id: 'ended',
      header: t('audit.columns.state'),
      meta: { label: t('audit.columns.state') },
      cell: ({ row }) =>
        row.original.ended_at == null ? t('audit.state.open') : t('audit.state.closed'),
    }),
  ])
}

/** Connections: who connected to which device, how, and for how long. */
export function ConnectionsPanel({ search, onSearchChange }: PanelProps) {
  const { t, i18n } = useTranslation()
  const params = auditListParams(search)
  const query = useListConnectionAudit(params, { query: { placeholderData: keepPreviousData } })
  const columns = useMemo(
    () =>
      connectionColumns(t, i18n.language, (id) => {
        onSearchChange({ item: id })
      }),
    [t, i18n.language, onSearchChange],
  )
  return (
    <AuditPanel
      id="audit-connections"
      label={t('audit.tabs.connections')}
      search={search}
      onSearchChange={onSearchChange}
      query={query}
      columns={columns}
      searchPlaceholder={t('audit.search.connections')}
      filters={
        <>
          <WindowFilter search={search} onChange={onSearchChange} />
          <TextFilter
            label={t('audit.filters.device')}
            value={search.device}
            onCommit={(device) => {
              onSearchChange({ device })
            }}
          />
          <UserFilter search={search} onChange={onSearchChange} />
          <SelectFilter
            label={t('audit.filters.connType')}
            value={search.conn_type}
            options={CONN_TYPE_CODES.map((code) => ({
              value: code,
              label: connTypeLabel(t, code),
            }))}
            onChange={(conn_type) => {
              onSearchChange({ conn_type })
            }}
          />
        </>
      }
      empty={{
        icon: Cable,
        title: t('audit.empty.connections.title'),
        description: t('audit.empty.connections.description'),
      }}
      detailTitle={(item) => t('audit.detail.connectionTitle', { device: item.device_id })}
      detail={(item) => <ConnectionDetail item={item} />}
    />
  )
}

function ConnectionDetail({ item }: { item: ConnectionAuditView }) {
  const { t } = useTranslation()
  return (
    <>
      <Fields>
        <Field name={t('audit.columns.device')}>{item.device_id}</Field>
        <Field name={t('audit.detail.received')}>
          <Time at={item.created_at} />
        </Field>
        <Field name={t('audit.columns.controller')}>
          {item.controller?.username ?? t('audit.noController')}
        </Field>
        <Field name={t('audit.detail.sessionId')}>
          {item.session_id ?? (
            <span className="text-muted-foreground">{t('audit.noSessionId')}</span>
          )}
        </Field>
        <Field name={t('audit.detail.connId')}>{item.conn_id}</Field>
      </Fields>
      <SelfReported>
        <Fields>
          <Field name={t('audit.columns.peer')}>
            <Optional value={item.peer_name} />
          </Field>
          <Field name={t('audit.detail.peerId')}>
            <Optional value={item.peer_id} />
          </Field>
          <Field name={t('audit.columns.type')}>
            {item.conn_type == null ? <Optional value={null} /> : connTypeLabel(t, item.conn_type)}
          </Field>
          <Field name={t('audit.detail.primaryAuth')}>
            {item.primary_auth == null ? (
              <Optional value={null} />
            ) : (
              primaryAuthLabel(t, item.primary_auth)
            )}
          </Field>
          <Field name={t('audit.detail.twoFactor')}>
            {item.two_factor == null ? t('audit.detail.none') : twoFactorLabel(t, item.two_factor)}
          </Field>
          <Field name={t('audit.columns.ip')}>
            <Optional value={item.ip} />
          </Field>
          <Field name={t('audit.detail.startedAt')}>
            <Time at={item.started_at} />
          </Field>
          <Field name={t('audit.detail.authorizedAt')}>
            <Time at={item.authorized_at} />
          </Field>
          <Field name={t('audit.detail.endedAt')}>
            <Time at={item.ended_at} />
          </Field>
          <Field name={t('audit.detail.note')}>
            <Optional value={item.note} />
          </Field>
          <Field name={t('audit.detail.controllerNote')}>
            <Optional value={item.controller_note} />
          </Field>
        </Fields>
      </SelfReported>
    </>
  )
}
