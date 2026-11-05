import { keepPreviousData } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { KeyboardEvent, Ref } from 'react'

import { useListUsers } from '@/api/generated/users/users'
import type { ListUsersParams, UserView } from '@/api/generated/model'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
import { Input } from '@/components/ui/input'
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
 * accounts by username; typing narrows them. Arrow keys move through the suggestions, Enter
 * or a click picks one. Enter picks only once the list answers the text as typed, so a fast
 * typist never gets a stale suggestion. Enter with text in the field never submits a
 * surrounding form, and Escape closes the list and clears the text without closing the
 * drawer or dialog around it.
 */
export function UserCombobox(props: UserComboboxProps) {
  const { label, listLabel, placeholder, filter, busy = false, disabled = false } = props
  const { foundText, noMatchText, onPick, inputRef } = props
  const listboxId = useId()
  const [q, setQ] = useState('')
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const term = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const candidates = useListUsers(
    { ...filter, q: term, page_size: CANDIDATES, sort: 'username' },
    { query: { enabled: open, placeholderData: keepPreviousData } },
  )
  const shown = open ? (candidates.data?.items ?? []) : []
  const expanded = shown.length > 0
  const current = shown[Math.min(active, shown.length - 1)]
  const settled = term === q.trim() && !candidates.isPlaceholderData

  const pick = async (user: UserView) => {
    if (busy || disabled) return
    const kept = (await onPick(user)) === false
    if (kept) return
    setQ('')
    setActive(0)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      // Typing a name and pressing Enter is a search, never a form submission.
      if (q.trim() !== '') event.preventDefault()
      if (expanded && settled && current !== undefined) void pick(current)
    } else if (event.key === 'Escape' && (q !== '' || expanded)) {
      // Dismiss handlers of the sheet / dialog around listen on the document.
      event.preventDefault()
      event.stopPropagation()
      setQ('')
      setOpen(false)
    } else if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && expanded) {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      setActive((index) => (index + step + shown.length) % shown.length)
    }
  }

  return (
    <div className="grid gap-1">
      <Input
        ref={inputRef}
        role="combobox"
        aria-label={label}
        aria-expanded={expanded}
        aria-controls={listboxId}
        aria-autocomplete="list"
        aria-activedescendant={current ? `${listboxId}-${current.id}` : undefined}
        aria-busy={busy || (open && !settled)}
        disabled={disabled}
        placeholder={placeholder ?? label}
        value={q}
        onFocus={() => {
          setOpen(true)
        }}
        onBlur={() => {
          setOpen(false)
        }}
        onChange={(e) => {
          setQ(e.target.value)
          setOpen(true)
          setActive(0)
        }}
        onKeyDown={onKeyDown}
      />
      <ul
        id={listboxId}
        role="listbox"
        aria-label={listLabel}
        hidden={!expanded}
        className="grid gap-0.5 rounded-lg border p-1 text-sm"
      >
        {shown.map((user) => (
          <li
            key={user.id}
            id={`${listboxId}-${user.id}`}
            role="option"
            aria-selected={user.id === current?.id}
            aria-disabled={busy}
            className={cn(
              'cursor-pointer rounded-md px-2 py-1 aria-disabled:cursor-wait aria-disabled:opacity-50',
              user.id === current?.id && 'bg-muted',
            )}
            // Mouse down, not click: the input keeps focus (and its keyboard handling).
            onMouseDown={(event) => {
              event.preventDefault()
              void pick(user)
            }}
          >
            {user.username}
            <span className="text-muted-foreground"> · {user.display_name}</span>
          </li>
        ))}
      </ul>
      <p role="status" className="text-xs text-muted-foreground">
        {!open || candidates.data === undefined
          ? null
          : shown.length === 0
            ? noMatchText
            : foundText(shown.length)}
      </p>
    </div>
  )
}
