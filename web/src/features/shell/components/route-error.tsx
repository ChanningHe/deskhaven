import { useQueryErrorResetBoundary } from '@tanstack/react-query'
import { Link, useRouter } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import { ShieldX, TriangleAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { errorCodeOf, errorMessage } from '@/api/error-message'
import { EmptyState } from '@/components/common/empty-state'
import { Button, buttonVariants } from '@/components/ui/button'
import { httpStatusOf } from '@/lib/http-error'

/**
 * Missing permission (route guard, or the API's `forbidden`): retrying cannot
 * help, so only offer a way out. Other 403s (`csrf_failed`) get their message.
 */
function Forbidden() {
  const { t } = useTranslation()
  return (
    <div role="alert" className="py-6">
      <EmptyState
        icon={ShieldX}
        headingLevel={1}
        title={t('errors.forbidden.title')}
        description={t('errors.forbidden.description')}
        action={
          <Link to="/" className={buttonVariants({ variant: 'outline' })}>
            {t('errors.home')}
          </Link>
        }
      />
    </div>
  )
}

/**
 * Default error boundary for every route. Raw error messages are not shown:
 * they are untranslated and may contain server internals; API errors are
 * described by their translated code.
 */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const { t } = useTranslation()
  const router = useRouter()
  const queryErrorBoundary = useQueryErrorResetBoundary()

  if (errorCodeOf(error) === 'forbidden') return <Forbidden />

  const retry = () => {
    queryErrorBoundary.reset()
    reset()
    void router.invalidate()
  }
  const status = httpStatusOf(error)

  return (
    <div role="alert" className="py-6">
      <EmptyState
        icon={TriangleAlert}
        headingLevel={1}
        title={t('errors.route.title')}
        description={status === undefined ? t('errors.route.description') : errorMessage(error, t)}
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
