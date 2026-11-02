import { Link } from '@tanstack/react-router'
import { Monitor } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { buttonVariants } from '@/components/ui/button'

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
          // A link styled as a button keeps the link role (Base UI's Button
          // would stamp role="button" onto the <a>).
          <Link to="/client-setup" className={buttonVariants({ variant: 'outline' })}>
            {t('pages.devices.emptyAction')}
          </Link>
        }
      />
    </div>
  )
}
