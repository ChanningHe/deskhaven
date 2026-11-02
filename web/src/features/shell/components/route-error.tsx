import { useQueryErrorResetBoundary } from '@tanstack/react-query'
import { Link, useRouter } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import { TriangleAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { Button, buttonVariants } from '@/components/ui/button'

/**
 * Default error boundary for every route. Raw error messages are not shown:
 * they are untranslated and may contain server internals.
 */
export function RouteError({ reset }: ErrorComponentProps) {
  const { t } = useTranslation()
  const router = useRouter()
  const queryErrorBoundary = useQueryErrorResetBoundary()

  const retry = () => {
    queryErrorBoundary.reset()
    reset()
    void router.invalidate()
  }

  return (
    <div role="alert" className="py-6">
      <EmptyState
        icon={TriangleAlert}
        headingLevel={1}
        title={t('errors.route.title')}
        description={t('errors.route.description')}
        action={
          <>
            <Button onClick={retry}>{t('errors.retry')}</Button>
            <Link to="/" className={buttonVariants({ variant: 'outline' })}>
              {t('errors.home')}
            </Link>
          </>
        }
      />
    </div>
  )
}
