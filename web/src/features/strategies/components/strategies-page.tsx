import { SlidersHorizontal } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function StrategiesPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.strategies')} description={t('pages.strategies.description')} />
      <EmptyState
        icon={SlidersHorizontal}
        title={t('pages.strategies.emptyTitle')}
        description={t('pages.strategies.emptyDescription')}
      />
    </div>
  )
}
