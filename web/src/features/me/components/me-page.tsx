import { useTranslation } from 'react-i18next'

import { useGetMyAccount } from '@/api/generated/me/me'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'

import { BooksTab } from './books-tab'
import { DevicesTab } from './devices-tab'
import { PasswordCard } from './password-card'
import { ProfileCard } from './profile-card'
import { SessionsTab } from './sessions-tab'
import { TwoFactorCard } from './two-factor-card'

/** `/me`: the signed-in user's own account (permission `self`: every role). */
export function MePage() {
  const { t } = useTranslation()
  const query = useGetMyAccount()
  const account = query.data

  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.me')} description={t('pages.me.description')} />
      <Tabs defaultValue="account">
        <TabsList>
          <TabsTab value="account">{t('me.tabs.account')}</TabsTab>
          <TabsTab value="sessions">{t('me.tabs.sessions')}</TabsTab>
          <TabsTab value="devices">{t('me.tabs.devices')}</TabsTab>
          <TabsTab value="books">{t('me.tabs.books')}</TabsTab>
        </TabsList>
        <TabsPanel value="account">
          {query.isError ? (
            <ErrorAlert
              error={query.error}
              title={t('me.loadFailed')}
              onRetry={() => void query.refetch()}
            />
          ) : null}
          {account === undefined ? (
            query.isError ? null : (
              <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
            )
          ) : (
            <>
              <ProfileCard account={account} />
              <PasswordCard account={account} />
              <TwoFactorCard account={account} />
            </>
          )}
        </TabsPanel>
        <TabsPanel value="sessions">
          <SessionsTab />
        </TabsPanel>
        <TabsPanel value="devices">
          <DevicesTab />
        </TabsPanel>
        <TabsPanel value="books">
          <BooksTab />
        </TabsPanel>
      </Tabs>
    </div>
  )
}
