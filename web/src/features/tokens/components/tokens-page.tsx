import { KeyRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

export function TokensPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.tokens')} description={t('pages.tokens.description')} />
      <EmptyState
        icon={KeyRound}
        title={t('pages.tokens.emptyTitle')}
        description={t('pages.tokens.emptyDescription')}
      />
    </div>
  )
}
