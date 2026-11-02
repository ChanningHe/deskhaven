import { Users } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function UsersPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.users')} description={t('pages.users.description')} />
      <EmptyState
        icon={Users}
        title={t('pages.users.emptyTitle')}
        description={t('pages.users.emptyDescription')}
      />
    </div>
  )
}
