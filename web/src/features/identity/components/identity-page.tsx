import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PageHeader } from '@/components/common/page-header'
import { UnsavedGuard } from '@/components/common/unsaved-guard'
import { hasPermission } from '@/features/auth/permissions'
import { useMe } from '@/features/auth/session'

import { LdapSection } from './ldap-section'
import { OidcSection } from './oidc-section'
import { TfaSection } from './tfa-section'

/** Identity configuration (08 §3.2): OIDC providers, the LDAP directory, the 2FA policy. */
export function IdentityPage() {
  const { t } = useTranslation()
  const me = useMe()
  const isOwner = hasPermission(me.role, 'owner')
  const [ldapDirty, setLdapDirty] = useState(false)
  const [tfaDirty, setTfaDirty] = useState(false)
  return (
    <div className="space-y-8">
      <PageHeader title={t('nav.identity')} description={t('pages.identity.description')} />
      <div className="space-y-6">
        <OidcSection isOwner={isOwner} />
        <LdapSection isOwner={isOwner} onDirtyChange={setLdapDirty} />
        <TfaSection isOwner={isOwner} onDirtyChange={setTfaDirty} />
      </div>
      {/* One guard for the page: two would each ask, one after the other. */}
      <UnsavedGuard dirty={ldapDirty || tfaDirty} />
    </div>
  )
}
