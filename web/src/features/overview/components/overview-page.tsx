import { LayoutDashboard } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function OverviewPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.overview')} description={t('pages.overview.description')} />
      <EmptyState
        icon={LayoutDashboard}
        title={t('pages.overview.emptyTitle')}
        description={t('pages.overview.emptyDescription')}
      />
    </div>
  )
}
