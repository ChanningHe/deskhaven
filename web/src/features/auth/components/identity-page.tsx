import { Fingerprint } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function IdentityPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.identity')} description={t('pages.identity.description')} />
      <EmptyState
        icon={Fingerprint}
        title={t('pages.identity.emptyTitle')}
        description={t('pages.identity.emptyDescription')}
      />
    </div>
  )
}
