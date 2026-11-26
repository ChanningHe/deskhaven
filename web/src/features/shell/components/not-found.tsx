import { Link } from '@tanstack/react-router'
import { Compass } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { buttonVariants } from '@/components/ui/button'

export function NotFound() {
  const { t } = useTranslation()
  return (
    <EmptyState
      icon={Compass}
      headingLevel={1}
      title={t('notFound.title')}
      description={t('notFound.description')}
      action={
        <Link to="/" className={buttonVariants({ variant: 'outline' })}>
          {t('notFound.back')}
        </Link>
      }
    />
  )
}

export function NotFoundPage() {
  return (
    <main className="mx-auto flex min-h-svh max-w-lg items-center px-4">
      <div className="w-full">
        <NotFound />
      </div>
    </main>
  )
}
