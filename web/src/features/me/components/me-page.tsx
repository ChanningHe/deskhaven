import { UserRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

// Placeholder until M3-13 (profile, password, 2FA, sessions).
export function MePage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.me')} description={t('pages.me.description')} />
      <EmptyState
        icon={UserRound}
        title={t('pages.me.emptyTitle')}
        description={t('pages.me.emptyDescription')}
      />
    </div>
  )
}
