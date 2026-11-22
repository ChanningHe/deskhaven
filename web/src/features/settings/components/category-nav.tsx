import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

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
      className="sticky top-0 z-10 min-w-0 bg-background/95 py-2 backdrop-blur xl:top-6 xl:order-last xl:self-start xl:py-0"
    >
      <ul className="flex gap-1 overflow-x-auto xl:flex-col xl:overflow-visible">
        {items.map((item) => (
          <li key={item.name} className="shrink-0">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={
                item.unsaved > 0
                  ? t('settings.nav.unsaved', { label: item.label, count: item.unsaved })
                  : undefined
              }
              className="xl:w-full xl:justify-between"
              onClick={() => {
                onJump(item.name)
              }}
            >
              {item.label}
              {item.unsaved > 0 ? (
                <span
                  aria-hidden
                  className="rounded-full bg-info/10 px-1.5 text-xs font-medium text-info tabular-nums"
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
