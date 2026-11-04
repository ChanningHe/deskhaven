import { useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'

import type { SearchChangeOptions } from '@/components/common/data-table'
import { PageHeader } from '@/components/common/page-header'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'

import { applyAuditSearch, AUDIT_TABS } from '../audit-search'
import type { AuditSearch, AuditTab } from '../audit-search'
import { AlarmsPanel } from './alarms-panel'
import { ConnectionsPanel } from './connections-panel'
import { ConsolePanel } from './console-panel'
import { FilesPanel } from './files-panel'
import { LoginsPanel } from './logins-panel'

/** What the device reports is not verified by the server (06 §2): said once, above the tabs. */
const REPORTED_BY_DEVICES: readonly AuditTab[] = ['connections', 'files', 'alarms']

/** Audit: one tab per kind of record; the tab, filters and open record are in the URL. */
export function AuditPage() {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/audit' })
  const navigate = useNavigate({ from: '/audit' })
  const tab: AuditTab = search.tab ?? 'connections'

  const onSearchChange = useCallback(
    (patch: Partial<AuditSearch>, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: AuditSearch) => applyAuditSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  const panel = { search, onSearchChange }

  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.audit')} description={t('pages.audit.description')} />
      <Tabs
        value={tab}
        onValueChange={(next: AuditTab) => {
          // A tab starts with its own list: page, sort, filters and drawer do not carry over;
          // the time window does, it means the same on every tab.
          void navigate({
            search: {
              ...(next === 'connections' ? {} : { tab: next }),
              ...(search.from === undefined ? {} : { from: search.from }),
              ...(search.to === undefined ? {} : { to: search.to }),
            },
          })
        }}
      >
        <TabsList aria-label={t('audit.tabs.label')}>
          {AUDIT_TABS.map((value) => (
            <TabsTab key={value} value={value}>
              {t(`audit.tabs.${value}`)}
            </TabsTab>
          ))}
        </TabsList>
        {REPORTED_BY_DEVICES.includes(tab) ? (
          <p className="text-sm text-muted-foreground">{t('audit.selfReportedNote')}</p>
        ) : null}
        <TabsPanel value="connections">
          <ConnectionsPanel {...panel} />
        </TabsPanel>
        <TabsPanel value="files">
          <FilesPanel {...panel} />
        </TabsPanel>
        <TabsPanel value="alarms">
          <AlarmsPanel {...panel} />
        </TabsPanel>
        <TabsPanel value="logins">
          <LoginsPanel {...panel} />
        </TabsPanel>
        <TabsPanel value="console">
          <ConsolePanel {...panel} />
        </TabsPanel>
      </Tabs>
    </div>
  )
}
