import { Search } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

// Placeholder for the ⌘K command palette (08 §3.1); not interactive yet.
export function SearchButton() {
  const { t } = useTranslation()
  return (
    <Button
      variant="outline"
      className="w-full max-w-64 justify-start text-muted-foreground"
      aria-keyshortcuts="Meta+K Control+K"
      title={t('shell.searchUnavailable')}
      disabled
    >
      <Search aria-hidden />
      <span className="flex-1 text-left">{t('shell.search')}</span>
      <kbd className="hidden rounded border bg-muted px-1.5 font-sans text-[0.7rem] sm:inline">
        ⌘K
      </kbd>
    </Button>
  )
}
