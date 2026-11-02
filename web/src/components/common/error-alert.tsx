import { CircleAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { errorMessage } from '@/api/error-message'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

interface ErrorAlertProps {
  error: unknown
  /** Short context, e.g. "Could not load the list"; the code's message follows. */
  title?: string
  onRetry?: () => void
  className?: string
}

/** An inline, translated error (by API error code) with an optional retry. */
export function ErrorAlert({ error, title, onRetry, className }: ErrorAlertProps) {
  const { t } = useTranslation()
  return (
    <div
      role="alert"
      className={cn(
        'flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive',
        className,
      )}
    >
      <CircleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1">
        {title === undefined ? null : <p className="font-medium">{title}</p>}
        <p>{errorMessage(error, t)}</p>
      </div>
      {onRetry === undefined ? null : (
        <Button variant="outline" size="sm" onClick={onRetry}>
          {t('errors.retry')}
        </Button>
      )}
    </div>
  )
}
