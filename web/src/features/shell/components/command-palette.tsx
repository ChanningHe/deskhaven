import { Autocomplete } from '@base-ui/react/autocomplete'
import { Dialog } from '@base-ui/react/dialog'
import { Search } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { IS_APPLE_PLATFORM, isOtherDialogOpen, isPaletteShortcut } from '../command-shortcut'
import { useCommandGroups } from '../use-command-groups'
import type { CommandGroup, CommandItem } from '../use-command-groups'

function PaletteList({ onRun }: { onRun: (item: CommandItem) => void }) {
  const { t } = useTranslation()
  const groups = useCommandGroups()
  const hintId = useId()

  return (
    <Autocomplete.Root open inline items={groups} autoHighlight="always" keepHighlight>
      <div className="flex items-center gap-2 border-b px-3">
        <Search aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        <Autocomplete.Input
          aria-label={t('shell.commandSearch')}
          aria-describedby={hintId}
          placeholder={t('shell.commandPlaceholder')}
          className="h-11 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
      </div>
      <div className="max-h-[min(60dvh,24rem)] overflow-y-auto overscroll-contain p-1">
        <Autocomplete.Empty className="px-3 py-6 text-center text-sm text-muted-foreground empty:hidden">
          {t('shell.commandEmpty')}
        </Autocomplete.Empty>
        <Autocomplete.List>
          {(group: CommandGroup) => (
            <Autocomplete.Group key={group.value} items={group.items} className="not-last:mb-1">
              <Autocomplete.GroupLabel className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                {group.value}
              </Autocomplete.GroupLabel>
              <Autocomplete.Collection>
                {(item: CommandItem) => (
                  <Autocomplete.Item
                    key={item.value}
                    value={item}
                    onClick={() => {
                      onRun(item)
                    }}
                    className="flex h-8 cursor-default items-center rounded-md px-2 text-sm outline-none select-none data-highlighted:bg-accent data-highlighted:text-accent-foreground"
                  >
                    {item.label}
                  </Autocomplete.Item>
                )}
              </Autocomplete.Collection>
            </Autocomplete.Group>
          )}
        </Autocomplete.List>
      </div>
      <p id={hintId} className="border-t px-3 py-2 text-xs text-muted-foreground">
        {t('shell.commandHint')}
      </p>
    </Autocomplete.Root>
  )
}

/**
 * ⌘K command palette (08 §3.1): the top-bar search button opens it, and so
 * does ⌘K / Ctrl+K from anywhere in the console.
 */
export function CommandPalette() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isPaletteShortcut(event)) return
      // Never stack on top of another modal (a confirmation, the mobile drawer).
      if (!open && isOtherDialogOpen()) return
      event.preventDefault()
      setOpen(!open)
    }
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  const run = (item: CommandItem) => {
    setOpen(false)
    item.run()
  }

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Trigger
        aria-keyshortcuts="Meta+K Control+K"
        className="flex h-8 w-full max-w-64 items-center gap-1.5 rounded-lg border px-2.5 text-sm text-muted-foreground transition-colors outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Search aria-hidden className="size-4 shrink-0" />
        <span className="truncate">{t('shell.search')}</span>
        <kbd
          aria-hidden
          className="ml-auto hidden rounded border bg-muted px-1.5 font-mono text-[0.625rem] sm:inline"
        >
          {IS_APPLE_PLATFORM ? '⌘K' : 'Ctrl K'}
        </kbd>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Backdrop className="fixed inset-0 z-50 bg-black/10 supports-backdrop-filter:backdrop-blur-xs" />
        <Dialog.Viewport className="fixed inset-0 z-50 flex items-start justify-center px-4 pt-[12vh]">
          <Dialog.Popup
            aria-label={t('shell.commandPalette')}
            className="w-full max-w-lg overflow-hidden rounded-xl bg-popover text-popover-foreground shadow-lg ring-1 ring-foreground/10 outline-none"
          >
            <PaletteList onRun={run} />
          </Dialog.Popup>
        </Dialog.Viewport>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
