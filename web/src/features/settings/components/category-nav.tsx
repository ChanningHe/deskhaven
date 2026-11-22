import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

export interface CategoryNavItem {
  name: string
  label: string
  /** How many settings of the category have an unsaved draft. */
  unsaved: number
}

interface CategoryNavProps {
  items: readonly CategoryNavItem[]
  onJump: (name: string) => void
}

/**
 * Jumps within the page to a category. Buttons, not links: changing the URL (even only its
 * hash) would trip the unsaved-changes guard on every click while there are drafts.
 */
export function CategoryNav({ items, onJump }: CategoryNavProps) {
  const { t } = useTranslation()
  return (
    <nav
      aria-label={t('settings.nav.label')}
      className="sticky top-0 z-10 -mx-4 min-w-0 border-b bg-background/95 backdrop-blur sm:-mx-6 lg:-mx-8 xl:top-6 xl:order-last xl:mx-0 xl:self-start xl:border-0 xl:bg-transparent xl:backdrop-blur-none"
    >
      <p
        aria-hidden
        className="hidden px-3 pb-2 text-xs font-medium text-muted-foreground xl:block"
      >
        {t('settings.nav.label')}
      </p>
      <ul className="flex [scrollbar-width:thin] gap-2 overflow-x-auto px-4 py-2.5 sm:px-6 lg:px-8 xl:flex-col xl:gap-0.5 xl:overflow-visible xl:p-0">
        {items.map((item) => (
          <li key={item.name} className="shrink-0">
            <Button
              type="button"
              variant="ghost"
              aria-label={
                item.unsaved > 0
                  ? t('settings.nav.unsaved', { label: item.label, count: item.unsaved })
                  : undefined
              }
              className={cn(
                'rounded-full border-border px-3 font-normal xl:w-full xl:justify-between xl:rounded-lg xl:border-transparent',
                item.unsaved > 0 ? 'xl:text-foreground' : 'xl:text-muted-foreground',
              )}
              onClick={() => {
                onJump(item.name)
              }}
            >
              {item.label}
              {item.unsaved > 0 ? (
                <span
                  aria-hidden
                  className="inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-info/10 px-1.5 text-xs font-medium text-info tabular-nums"
                >
                  {item.unsaved}
                </span>
              ) : null}
            </Button>
          </li>
        ))}
      </ul>
    </nav>
  )
}
