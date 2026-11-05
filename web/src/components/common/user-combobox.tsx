import { Autocomplete } from '@base-ui/react/autocomplete'
import type { BaseUIEvent } from '@base-ui/react/types'
import { keepPreviousData } from '@tanstack/react-query'
import { useState } from 'react'
import type { KeyboardEvent, Ref } from 'react'

import { useListUsers } from '@/api/generated/users/users'
import type { ListUsersParams, UserView } from '@/api/generated/model'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
import {
  AutocompleteContent,
  AutocompleteInput,
  AutocompleteItem,
  AutocompleteList,
  AutocompleteStatus,
} from '@/components/ui/autocomplete'
import { useDebouncedValue } from '@/lib/use-debounced-value'
import { cn } from '@/lib/utils'

/** Candidates offered at once; typing more narrows them. */
const CANDIDATES = 8

export interface UserComboboxProps {
  /** Accessible name of the search field. */
  label: string
  /** Accessible name of the list of suggestions. */
  listLabel: string
  placeholder?: string
  /** Narrows the search (e.g. `not_group`); `q`, `page_size` and `sort` are the combobox's. */
  filter?: Omit<ListUsersParams, 'q' | 'page_size' | 'sort' | 'page'>
  /** An earlier pick is still being handled: picks are ignored meanwhile. */
  busy?: boolean
  disabled?: boolean
  /** Status line for `count` suggestions (screen readers announce it). */
  foundText: (count: number) => string
  noMatchText: string
  /** Handles the picked user; `false` keeps the typed text (the pick failed). */
  onPick: (user: UserView) => boolean | undefined | Promise<boolean | undefined>
  inputRef?: Ref<HTMLInputElement>
}

/**
 * A search-as-you-type combobox over the accounts. Focusing the field lists the first
 * accounts by username in a floating list that overlays the content around it; typing narrows
 * them. Arrow keys move through the suggestions, Enter or a click picks one. Enter picks only
 * once the list answers the text as typed, so a fast typist never gets a stale suggestion.
 * Enter with text in the field never submits a surrounding form, and Escape closes the list
 * and clears the text without closing the drawer or dialog around it.
 */
export function UserCombobox(props: UserComboboxProps) {
  const { label, listLabel, placeholder, filter, busy = false, disabled = false } = props
  const { foundText, noMatchText, onPick, inputRef } = props
  const [q, setQ] = useState('')
  const [open, setOpen] = useState(false)
  const term = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const candidates = useListUsers(
    { ...filter, q: term, page_size: CANDIDATES, sort: 'username' },
    { query: { enabled: open, placeholderData: keepPreviousData } },
  )
  const shown = open ? (candidates.data?.items ?? []) : []
  const expanded = shown.length > 0
  // Open while empty too: the popup carries the status line, which the dialog around would
  // otherwise hide from screen readers as long as the field is expanded.
  const popupOpen = open && candidates.data !== undefined
  const settled = term === q.trim() && !candidates.isPlaceholderData

  const pick = async (user: UserView) => {
    if (busy || disabled) return
    const kept = (await onPick(user)) === false
    if (kept) {
      // The pick closed the list; the text is still there to retry with.
      setOpen(true)
    } else {
      setQ('')
    }
  }

  const onKeyDown = (event: BaseUIEvent<KeyboardEvent<HTMLInputElement>>) => {
    if (event.key === 'Enter') {
      // Typing a name and pressing Enter is a search, never a form submission.
      if (q.trim() !== '') event.preventDefault()
      // Base UI picks the highlighted suggestion; hold that back while it answers older text.
      if (!expanded || !settled) event.preventBaseUIHandler()
    } else if (event.key === 'Escape' && (q !== '' || popupOpen)) {
      // Dismiss handlers of the sheet / dialog around listen on the document.
      event.preventBaseUIHandler()
      event.preventDefault()
      event.stopPropagation()
      setQ('')
      setOpen(false)
    }
  }

  return (
    <div className="grid gap-1">
      <Autocomplete.Root
        items={shown}
        // The server filters; the list shows what it answered.
        filter={null}
        autoHighlight="always"
        openOnInputClick
        disabled={disabled}
        open={popupOpen}
        onOpenChange={setOpen}
        value={q}
        onValueChange={(next, details) => {
          // Base UI would copy the picked name into the field; `pick` clears it instead.
          if (details.reason !== 'item-press') setQ(next)
        }}
        itemToStringValue={(user: UserView) => user.username}
      >
        <AutocompleteInput
          ref={inputRef}
          aria-label={label}
          aria-busy={busy || (open && !settled)}
          placeholder={placeholder ?? label}
          onFocus={() => {
            setOpen(true)
          }}
          onKeyDown={onKeyDown}
        />
        <AutocompleteContent>
          <AutocompleteStatus className={cn(expanded && 'sr-only')}>
            {expanded ? foundText(shown.length) : noMatchText}
          </AutocompleteStatus>
          <AutocompleteList aria-label={listLabel}>
            {(user: UserView) => (
              <AutocompleteItem
                key={user.id}
                value={user}
                disabled={busy}
                onClick={() => {
                  void pick(user)
                }}
              >
                {user.username}
                <span className="text-muted-foreground"> · {user.display_name}</span>
              </AutocompleteItem>
            )}
          </AutocompleteList>
        </AutocompleteContent>
      </Autocomplete.Root>
    </div>
  )
}
