import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import { LogoMark } from './logo-mark'

interface BrandProps {
  /** Icon only; the name stays available to assistive technology. */
  compact?: boolean
}

export function Brand({ compact = false }: BrandProps) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2 font-semibold tracking-tight">
      {/* the app icon (assets/logo/icon.svg): the mark at 72% on a rounded square */}
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
        <LogoMark aria-hidden className="size-5" />
      </span>
      <span className={cn(compact && 'sr-only')}>{t('app.name')}</span>
    </div>
  )
}
