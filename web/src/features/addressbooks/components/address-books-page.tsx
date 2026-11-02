import { BookUser } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function AddressBooksPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.addressBooks')} description={t('pages.addressBooks.description')} />
      <EmptyState
        icon={BookUser}
        title={t('pages.addressBooks.emptyTitle')}
        description={t('pages.addressBooks.emptyDescription')}
      />
    </div>
  )
}
