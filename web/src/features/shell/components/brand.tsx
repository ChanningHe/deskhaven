import { Anchor } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

interface BrandProps {
  /** Icon only; the name stays available to assistive technology. */
  compact?: boolean
}

export function Brand({ compact = false }: BrandProps) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2 font-semibold tracking-tight">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
        <Anchor aria-hidden className="size-4" />
      </span>
      <span className={cn(compact && 'sr-only')}>{t('app.name')}</span>
    </div>
  )
}
