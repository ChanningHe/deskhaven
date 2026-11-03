import { useNavigate, useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { PageHeader } from '@/components/common/page-header'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'
import { DeviceGroupsTab } from '@/features/device-groups/components/device-groups-tab'

import type { GroupsTab } from '../groups-search'
import { UserGroupsTab } from './user-groups-tab'

/** Groups: user groups and device groups, one tab each (the tab is in the URL). */
export function GroupsPage() {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/groups' })
  const navigate = useNavigate({ from: '/groups' })
  const tab: GroupsTab = search.tab ?? 'users'
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.groups')} description={t('pages.groups.description')} />
      <Tabs
        value={tab}
        onValueChange={(next: GroupsTab) => {
          // A tab starts with its own list: page, sort, filter and drawer do not carry over.
          void navigate({ search: next === 'devices' ? { tab: 'devices' } : {} })
        }}
      >
        <TabsList aria-label={t('groups.tabs.label')}>
          <TabsTab value="users">{t('groups.tabs.users')}</TabsTab>
          <TabsTab value="devices">{t('groups.tabs.devices')}</TabsTab>
        </TabsList>
        <TabsPanel value="users">
          <UserGroupsTab />
        </TabsPanel>
        <TabsPanel value="devices">
          <DeviceGroupsTab />
        </TabsPanel>
      </Tabs>
    </div>
  )
}
