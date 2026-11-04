import { keepPreviousData } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { FileUp } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListFileAudit } from '@/api/generated/audit/audit'
import type { FileAuditView } from '@/api/generated/model'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { directionLabel } from '../audit-labels'
import { auditListParams, DIRECTIONS } from '../audit-search'
import { SelectFilter, TextFilter, WindowFilter } from './audit-filters'
import { AuditPanel } from './audit-panel'
import type { PanelProps } from './audit-panel'
import { Field, Fields, Optional, SelfReported, Time } from './detail-fields'
import { OpenButton } from './open-button'

const column = createDataTableColumnHelper<FileAuditView>()

function fileColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
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
    column.accessor('peer_id', {
      header: t('audit.columns.peer'),
      meta: { label: t('audit.columns.peer'), className: 'font-mono' },
    }),
    column.accessor('direction', {
      header: t('audit.columns.direction'),
      meta: { label: t('audit.columns.direction') },
      cell: ({ row }) => directionLabel(t, row.original.direction),
    }),
    column.accessor('path', {
      header: t('audit.columns.path'),
      meta: { label: t('audit.columns.path'), className: 'max-w-72 truncate font-mono text-xs' },
    }),
    column.accessor('file_count', {
      header: t('audit.columns.files'),
      meta: { label: t('audit.columns.files') },
    }),
  ])
}

/** File transfers, as the controlled device reported them. */
export function FilesPanel({ search, onSearchChange }: PanelProps) {
  const { t, i18n } = useTranslation()
  const query = useListFileAudit(auditListParams(search), {
    query: { placeholderData: keepPreviousData },
  })
  const columns = useMemo(
    () =>
      fileColumns(t, i18n.language, (id) => {
        onSearchChange({ item: id })
      }),
    [t, i18n.language, onSearchChange],
  )
  return (
    <AuditPanel
      id="audit-files"
      label={t('audit.tabs.files')}
      search={search}
      onSearchChange={onSearchChange}
      query={query}
      columns={columns}
      searchPlaceholder={t('audit.search.files')}
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
            label={t('audit.filters.direction')}
            value={search.direction}
            options={DIRECTIONS.map((value) => ({ value, label: directionLabel(t, value) }))}
            onChange={(direction) => {
              onSearchChange({ direction })
            }}
          />
        </>
      }
      empty={{
        icon: FileUp,
        title: t('audit.empty.files.title'),
        description: t('audit.empty.files.description'),
      }}
      detailTitle={(item) => t('audit.detail.fileTitle', { device: item.device_id })}
      detail={(item) => <FileDetail item={item} />}
    />
  )
}

function FileDetail({ item }: { item: FileAuditView }) {
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
          <Field name={t('audit.detail.peerId')}>{item.peer_id}</Field>
          <Field name={t('audit.columns.direction')}>{directionLabel(t, item.direction)}</Field>
          <Field name={t('audit.columns.path')}>{item.path}</Field>
          <Field name={t('audit.detail.isFile')}>
            {item.is_file ? t('audit.detail.file') : t('audit.detail.folder')}
          </Field>
          <Field name={t('audit.columns.files')}>{item.file_count}</Field>
          <Field name={t('audit.columns.ip')}>
            <Optional value={item.ip} />
          </Field>
        </Fields>
        {item.files.length === 0 ? null : (
          <ul aria-label={t('audit.detail.fileList')} className="grid gap-1 text-sm">
            {item.files.map((file, index) => (
              // The device may list one name twice: position is part of the identity.
              <li key={`${String(index)}:${file.name}`} className="flex justify-between gap-4">
                <span className="min-w-0 break-words">{file.name}</span>
                <span className="text-muted-foreground">{formatBytes(file.size)}</span>
              </li>
            ))}
          </ul>
        )}
      </SelfReported>
    </>
  )
}

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const
const STEP = 1024

function formatBytes(size: number): string {
  let value = Math.max(0, size)
  let unit = 0
  while (value >= STEP && unit < UNITS.length - 1) {
    value /= STEP
    unit += 1
  }
  const digits = unit === 0 || value >= 10 ? 0 : 1
  return `${String(Number(value.toFixed(digits)))} ${UNITS[unit] ?? 'B'}`
}
