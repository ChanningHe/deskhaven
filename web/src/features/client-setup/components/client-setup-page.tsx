import { MonitorCog } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function ClientSetupPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.clientSetup')} description={t('pages.clientSetup.description')} />
      <EmptyState
        icon={MonitorCog}
        title={t('pages.clientSetup.emptyTitle')}
        description={t('pages.clientSetup.emptyDescription')}
      />
    </div>
  )
}
