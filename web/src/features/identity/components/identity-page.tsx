import { useNavigate, useSearch } from '@tanstack/react-router'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PageHeader } from '@/components/common/page-header'
import { UnsavedGuard } from '@/components/common/unsaved-guard'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'
import { hasPermission } from '@/features/auth/permissions'
import { useMe } from '@/features/auth/session'

import { IDENTITY_TABS } from '../identity-search'
import type { IdentityTab } from '../identity-search'
import { useTabSummaries } from '../use-tab-summaries'
import { LdapSection } from './ldap-section'
import { OidcSection } from './oidc-section'
import { TfaSection } from './tfa-section'

/**
 * Identity configuration (08 §3.2): OIDC providers, the LDAP directory, the 2FA policy, one
 * tab each (the tab is in the URL).
 */
export function IdentityPage() {
  const { t } = useTranslation()
  const me = useMe()
  const isOwner = hasPermission(me.role, 'owner')
  const search = useSearch({ from: '/_app/identity' })
  const navigate = useNavigate({ from: '/identity' })
  const tab: IdentityTab = search.tab ?? 'oidc'
  const summaries = useTabSummaries(isOwner)
  const [ldapDirty, setLdapDirty] = useState(false)
  const [tfaDirty, setTfaDirty] = useState(false)
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.identity')} description={t('pages.identity.description')} />
      <Tabs
        value={tab}
        onValueChange={(next: IdentityTab) => {
          void navigate({ search: next === 'oidc' ? {} : { tab: next } })
        }}
      >
        <TabsList aria-label={t('identity.tabs.label')} className="max-w-full">
          {IDENTITY_TABS.map((value) => (
            <SummaryTab
              key={value}
              value={value}
              label={t(`identity.tabs.${value}`)}
              summary={summaries[value]}
            />
          ))}
        </TabsList>
        <TabsPanel value="oidc">
          <OidcSection isOwner={isOwner} />
        </TabsPanel>
        <TabsPanel value="ldap">
          <LdapSection isOwner={isOwner} onDirtyChange={setLdapDirty} />
        </TabsPanel>
        <TabsPanel value="tfa">
          <TfaSection isOwner={isOwner} onDirtyChange={setTfaDirty} />
        </TabsPanel>
      </Tabs>
      {/* Inactive panels unmount, so leaving a dirty tab, like leaving the page, drops its
          draft: the one guard asks first. */}
      <UnsavedGuard dirty={ldapDirty || tfaDirty} />
    </div>
  )
}

interface SummaryTabProps {
  value: IdentityTab
  label: string
  summary: string | undefined
}

/**
 * The summary describes the tab rather than naming it, and is read even where it is hidden.
 * Where the tabs do not fit, the summaries shorten and the labels stay whole.
 */
function SummaryTab({ value, label, summary }: SummaryTabProps) {
  const id = useId()
  return (
    <TabsTab
      value={value}
      className="min-w-0"
      aria-describedby={summary === undefined ? undefined : id}
    >
      {label}
      {summary === undefined ? null : (
        <span
          id={id}
          aria-hidden
          className="ml-1.5 hidden min-w-0 truncate font-normal text-muted-foreground sm:inline"
        >
          {summary}
        </span>
      )}
    </TabsTab>
  )
}
