import { Settings } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function SettingsPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.settings')} description={t('pages.settings.description')} />
      <EmptyState
        icon={Settings}
        title={t('pages.settings.emptyTitle')}
        description={t('pages.settings.emptyDescription')}
      />
    </div>
  )
}
