import { keepPreviousData } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { BellRing } from 'lucide-react'
import { useMemo } from 'react'
import { Trans, useTranslation } from 'react-i18next'

import { useListAlarmAudit } from '@/api/generated/audit/audit'
import type { AlarmAuditView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { ALARM_TYPE_CODES, alarmTypeLabel } from '../audit-labels'
import { auditListParams } from '../audit-search'
import { SelectFilter, TextFilter, WindowFilter } from './audit-filters'
import { AuditPanel } from './audit-panel'
import type { PanelProps } from './audit-panel'
import { Field, Fields, SelfReported, Time } from './detail-fields'
import { OpenButton } from './open-button'

const column = createDataTableColumnHelper<AlarmAuditView>()

function alarmColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
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
      cell: ({ getValue }) => <bdi>{getValue()}</bdi>,
    }),
    column.accessor('typ', {
      header: t('audit.columns.type'),
      meta: { label: t('audit.columns.type') },
      cell: ({ getValue }) => alarmTypeLabel(t, getValue()),
    }),
    column.display({
      id: 'info',
      header: t('audit.columns.info'),
      meta: { label: t('audit.columns.info'), className: 'max-w-72 truncate font-mono text-xs' },
      cell: ({ row }) => (
        <bdi>{row.original.info.map((entry) => `${entry.key}=${entry.value}`).join(' ')}</bdi>
      ),
    }),
  ])
}

/** Alarms raised by devices (blocked addresses, repeated login attempts, ...). */
export function AlarmsPanel({ search, onSearchChange }: PanelProps) {
  const { t, i18n } = useTranslation()
  const query = useListAlarmAudit(auditListParams(search), {
    query: { placeholderData: keepPreviousData },
  })
  const columns = useMemo(
    () =>
      alarmColumns(t, i18n.language, (id) => {
        onSearchChange({ item: id })
      }),
    [t, i18n.language, onSearchChange],
  )
  return (
    <AuditPanel
      id="audit-alarms"
      label={t('audit.tabs.alarms')}
      search={search}
      onSearchChange={onSearchChange}
      query={query}
      columns={columns}
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
          <SelectFilter
            label={t('audit.filters.alarmType')}
            value={search.typ}
            options={ALARM_TYPE_CODES.map((code) => ({
              value: code,
              label: alarmTypeLabel(t, code),
            }))}
            onChange={(typ) => {
              onSearchChange({ typ })
            }}
          />
        </>
      }
      empty={{
        icon: BellRing,
        title: t('audit.empty.alarms.title'),
        description: t('audit.empty.alarms.description'),
      }}
      detailTitle={(item) => (
        <Trans
          i18nKey="audit.detail.alarmTitle"
          values={{ device: item.device_id }}
          components={{ bdi: <bdi /> }}
        />
      )}
      detail={(item) => <AlarmDetail item={item} />}
    />
  )
}

function AlarmDetail({ item }: { item: AlarmAuditView }) {
  const { t } = useTranslation()
  return (
    <>
      <Fields>
        <Field name={t('audit.columns.device')}>{item.device_id}</Field>
        <Field name={t('audit.detail.received')}>
          <Time at={item.created_at} />
        </Field>
      </Fields>
      <SelfReported>
        <Fields>
          <Field name={t('audit.columns.type')}>{alarmTypeLabel(t, item.typ)}</Field>
          <Field name={t('audit.detail.connId')}>{item.conn_id}</Field>
          {item.info.map((entry) => (
            <Field key={entry.key} name={<bdi>{entry.key}</bdi>}>
              {entry.value}
            </Field>
          ))}
        </Fields>
      </SelfReported>
    </>
  )
}
