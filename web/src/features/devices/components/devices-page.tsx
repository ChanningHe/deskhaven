import { Link } from '@tanstack/react-router'
import { Monitor } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { Button } from '@/components/ui/button'

export function DevicesPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.devices')} description={t('pages.devices.description')} />
      <EmptyState
        icon={Monitor}
        title={t('pages.devices.emptyTitle')}
        description={t('pages.devices.emptyDescription')}
        action={
          <Button variant="outline" nativeButton={false} render={<Link to="/client-setup" />}>
            {t('pages.devices.emptyAction')}
          </Button>
        }
      />
    </div>
  )
}
