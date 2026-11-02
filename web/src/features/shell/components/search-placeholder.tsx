import { Search } from 'lucide-react'
import { useTranslation } from 'react-i18next'

/**
 * Reserves the top-bar slot of the ⌘K command palette (08 §3.1). Purely
 * visual and hidden from assistive technology until the palette exists; it
 * then becomes a real button with aria-keyshortcuts.
 */
export function SearchPlaceholder() {
  const { t } = useTranslation()
  return (
    <div
      aria-hidden="true"
      className="flex h-8 w-full max-w-64 items-center gap-1.5 rounded-lg border px-2.5 text-sm text-muted-foreground select-none"
    >
      <Search className="size-4 shrink-0" />
      <span className="truncate">{t('shell.search')}</span>
    </div>
  )
}
