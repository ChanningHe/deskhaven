import { ScrollText } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function AuditPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.audit')} description={t('pages.audit.description')} />
      <EmptyState
        icon={ScrollText}
        title={t('pages.audit.emptyTitle')}
        description={t('pages.audit.emptyDescription')}
      />
    </div>
  )
}
