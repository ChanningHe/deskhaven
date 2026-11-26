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

const CANDIDATES = 8

export interface UserComboboxProps {
  label: string
  listLabel: string
  placeholder?: string
  filter?: Omit<ListUsersParams, 'q' | 'page_size' | 'sort' | 'page'>
  busy?: boolean
  disabled?: boolean
  foundText: (count: number) => string
  noMatchText: string
  /** `false` keeps the typed text (the pick failed). */
  onPick: (user: UserView) => boolean | undefined | Promise<boolean | undefined>
  inputRef?: Ref<HTMLInputElement>
}

/**
 * Focus alone never opens the list: while it is open Base UI hides the rest of the page from
 * assistive technology. Enter picks only once the list answers the text as typed.
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
    if (!kept) setQ('')
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
        open={popupOpen && !disabled}
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
          onKeyDown={onKeyDown}
        />
        <AutocompleteContent>
          <AutocompleteStatus className={expanded ? 'sr-only' : 'px-2 py-1'}>
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
