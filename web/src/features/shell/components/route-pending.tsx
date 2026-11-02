import { LoaderCircle } from 'lucide-react'
import { useTranslation } from 'react-i18next'

/** Default pending UI, shown when a route takes longer than the router's pendingMs. */
export function RoutePending() {
  const { t } = useTranslation()
  return (
    <div
      role="status"
      className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground"
    >
      <LoaderCircle aria-hidden className="size-4 animate-spin" />
      {t('common.loading')}
    </div>
  )
}
