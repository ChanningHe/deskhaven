import { X } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

interface BulkActionBarProps {
  count: number
  onClear: () => void
  children: ReactNode
}

/** Shown above the table while rows are selected. */
export function BulkActionBar({ count, onClear, children }: BulkActionBarProps) {
  const { t } = useTranslation()
  return (
    <div
      role="region"
      aria-label={t('dataTable.selected', { count })}
      className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/50 px-3 py-2 text-sm"
    >
      <span aria-live="polite" className="font-medium">
        {t('dataTable.selected', { count })}
      </span>
      <div className="flex flex-wrap items-center gap-2">{children}</div>
      <Button variant="ghost" size="sm" className="ml-auto" onClick={onClear}>
        <X aria-hidden />
        {t('dataTable.clearSelection')}
      </Button>
    </div>
  )
}
