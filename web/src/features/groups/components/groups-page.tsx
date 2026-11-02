import { Boxes } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function GroupsPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.groups')} description={t('pages.groups.description')} />
      <EmptyState
        icon={Boxes}
        title={t('pages.groups.emptyTitle')}
        description={t('pages.groups.emptyDescription')}
      />
    </div>
  )
}
