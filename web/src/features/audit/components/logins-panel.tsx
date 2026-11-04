import { keepPreviousData } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { LogIn } from 'lucide-react'
import { useMemo } from 'react'
import { Trans, useTranslation } from 'react-i18next'

import { useListLoginAudit } from '@/api/generated/audit/audit'
import type { LoginAuditView } from '@/api/generated/model'
import { Badge } from '@/components/ui/badge'
import { createDataTableColumnHelper } from '@/components/common/data-table'
import { formatDateTime } from '@/lib/format'

import { clientLabel, methodLabel, resultLabel } from '../audit-labels'
import { auditListParams, LOGIN_CLIENTS, LOGIN_METHODS, LOGIN_RESULTS } from '../audit-search'
import { SelectFilter, UserFilter, WindowFilter } from './audit-filters'
import { AuditPanel } from './audit-panel'
import type { PanelProps } from './audit-panel'
import { Field, Fields, Optional, SelfReported, Time } from './detail-fields'
import { OpenButton } from './open-button'

const column = createDataTableColumnHelper<LoginAuditView>()

function ResultBadge({ result }: { result: string }) {
  const { t } = useTranslation()
  return (
    <Badge variant={result === 'success' ? 'secondary' : 'outline'}>{resultLabel(t, result)}</Badge>
  )
}

function loginColumns(t: TFunction, language: string, onOpen: (id: number) => void) {
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
    column.accessor('username_attempted', {
      header: t('audit.columns.username'),
      meta: { label: t('audit.columns.username'), className: 'max-w-48 truncate' },
      cell: ({ getValue }) => <bdi>{getValue()}</bdi>,
    }),
    column.accessor('method', {
      header: t('audit.columns.method'),
      meta: { label: t('audit.columns.method') },
      cell: ({ getValue }) => methodLabel(t, getValue()),
    }),
    column.accessor('result', {
      header: t('audit.columns.result'),
      meta: { label: t('audit.columns.result') },
      cell: ({ getValue }) => <ResultBadge result={getValue()} />,
    }),
    column.accessor('client', {
      header: t('audit.columns.client'),
      meta: { label: t('audit.columns.client') },
      cell: ({ getValue }) => clientLabel(t, getValue()),
    }),
    column.accessor('ip', {
      header: t('audit.columns.ip'),
      meta: { label: t('audit.columns.ip'), className: 'font-mono text-xs' },
      cell: ({ getValue }) => <bdi>{getValue() ?? ''}</bdi>,
    }),
  ])
}

/** Login attempts of the console and of RustDesk clients, successful or not. */
export function LoginsPanel({ search, onSearchChange }: PanelProps) {
  const { t, i18n } = useTranslation()
  const query = useListLoginAudit(auditListParams(search), {
    query: { placeholderData: keepPreviousData },
  })
  const columns = useMemo(
    () =>
      loginColumns(t, i18n.language, (id) => {
        onSearchChange({ item: id })
      }),
    [t, i18n.language, onSearchChange],
  )
  return (
    <AuditPanel
      id="audit-logins"
      label={t('audit.tabs.logins')}
      search={search}
      onSearchChange={onSearchChange}
      query={query}
      columns={columns}
      searchPlaceholder={t('audit.search.logins')}
      filters={
        <>
          <WindowFilter search={search} onChange={onSearchChange} />
          <UserFilter search={search} onChange={onSearchChange} />
          <SelectFilter
            label={t('audit.filters.method')}
            value={search.method}
            options={LOGIN_METHODS.map((value) => ({ value, label: methodLabel(t, value) }))}
            onChange={(method) => {
              onSearchChange({ method })
            }}
          />
          <SelectFilter
            label={t('audit.filters.client')}
            value={search.client}
            options={LOGIN_CLIENTS.map((value) => ({ value, label: clientLabel(t, value) }))}
            onChange={(client) => {
              onSearchChange({ client })
            }}
          />
          <SelectFilter
            label={t('audit.filters.result')}
            value={search.result}
            options={LOGIN_RESULTS.map((value) => ({ value, label: resultLabel(t, value) }))}
            onChange={(result) => {
              onSearchChange({ result })
            }}
          />
        </>
      }
      empty={{
        icon: LogIn,
        title: t('audit.empty.logins.title'),
        description: t('audit.empty.logins.description'),
      }}
      detailTitle={(item) => (
        <Trans
          i18nKey="audit.detail.loginTitle"
          values={{ name: item.username_attempted }}
          components={{ bdi: <bdi /> }}
        />
      )}
      detail={(item) => <LoginDetail item={item} />}
    />
  )
}

function LoginDetail({ item }: { item: LoginAuditView }) {
  const { t } = useTranslation()
  return (
    <>
      <Fields>
        <Field name={t('audit.detail.when')}>
          <Time at={item.created_at} />
        </Field>
        <Field name={t('audit.detail.account')}>
          {item.user?.username ?? t('audit.noAccount')}
        </Field>
        <Field name={t('audit.columns.method')}>{methodLabel(t, item.method)}</Field>
        <Field name={t('audit.columns.result')}>
          <ResultBadge result={item.result} />
        </Field>
        <Field name={t('audit.columns.client')}>{clientLabel(t, item.client)}</Field>
        <Field name={t('audit.columns.ip')}>
          <Optional value={item.ip} />
        </Field>
      </Fields>
      <SelfReported>
        <Fields>
          <Field name={t('audit.columns.username')}>{item.username_attempted}</Field>
          <Field name={t('audit.detail.clientDevice')}>
            <Optional value={item.rustdesk_id} />
          </Field>
        </Fields>
      </SelfReported>
    </>
  )
}
