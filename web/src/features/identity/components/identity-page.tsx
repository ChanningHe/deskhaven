import { useTranslation } from 'react-i18next'

import { PageHeader } from '@/components/common/page-header'
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
  return (
    <div className="space-y-10">
      <PageHeader title={t('nav.identity')} description={t('pages.identity.description')} />
      <OidcSection />
      <LdapSection isOwner={isOwner} />
      <TfaSection isOwner={isOwner} />
    </div>
  )
}
