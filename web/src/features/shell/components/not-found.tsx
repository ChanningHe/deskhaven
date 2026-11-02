import { Link } from '@tanstack/react-router'
import { Compass } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { buttonVariants } from '@/components/ui/button'

/** Not-found content; rendered inside the console shell by the `_app` layout. */
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

/** Full-page variant for URLs outside the shell (root route). */
export function NotFoundPage() {
  return (
    <main className="mx-auto flex min-h-svh max-w-lg items-center px-4">
      <div className="w-full">
        <NotFound />
      </div>
    </main>
  )
}
