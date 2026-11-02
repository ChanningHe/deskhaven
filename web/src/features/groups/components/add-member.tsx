import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { addGroupMember } from '@/api/generated/groups/groups'
import { useListUsers } from '@/api/generated/users/users'
import type { UserView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Input } from '@/components/ui/input'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
import { invalidateAccounts } from '@/features/users/invalidate'
import { useDebouncedValue } from '@/lib/use-debounced-value'
import { cn } from '@/lib/utils'

/** Candidates offered at once; typing more narrows them. */
const CANDIDATES = 8

/**
 * "Add a member": a combobox searching the accounts outside the group (the server leaves
 * the members out, whatever page of them the drawer shows). Arrow keys move through the
 * suggestions, Enter or a click adds one.
 */
export function AddMember({ groupId }: { groupId: number }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const listboxId = useId()
  const [q, setQ] = useState('')
  const [active, setActive] = useState(0)
  const [adding, setAdding] = useState(false)
  const [error, setError] = useState<unknown>(undefined)
  const term = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const candidates = useListUsers(
    { q: term, not_group: groupId, page_size: CANDIDATES, sort: 'username' },
    { query: { enabled: term.length > 0 } },
  )
  const shown = term.length > 0 ? (candidates.data?.items ?? []) : []
  const expanded = shown.length > 0
  const current = shown[Math.min(active, shown.length - 1)]

  const add = async (user: UserView) => {
    if (adding) return
    setAdding(true)
    setError(undefined)
    try {
      await addGroupMember(groupId, { user_id: user.id })
      setQ('')
      setActive(0)
      await invalidateAccounts(queryClient)
    } catch (caught) {
      setError(caught)
    } finally {
      setAdding(false)
    }
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (!expanded) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      setActive((index) => (index + step + shown.length) % shown.length)
    } else if (event.key === 'Enter' && current !== undefined) {
      event.preventDefault()
      void add(current)
    } else if (event.key === 'Escape') {
      event.preventDefault()
      setQ('')
    }
  }

  return (
    <div className="grid gap-1">
      <Input
        role="combobox"
        aria-label={t('groups.members.addSearch')}
        aria-expanded={expanded}
        aria-controls={listboxId}
        aria-autocomplete="list"
        aria-activedescendant={current ? `${listboxId}-${current.id}` : undefined}
        aria-busy={adding}
        placeholder={t('groups.members.addSearch')}
        value={q}
        onChange={(e) => {
          setQ(e.target.value)
          setActive(0)
        }}
        onKeyDown={onKeyDown}
      />
      <ul
        id={listboxId}
        role="listbox"
        aria-label={t('groups.members.candidates')}
        hidden={!expanded}
        className="grid gap-0.5 rounded-lg border p-1 text-sm"
      >
        {shown.map((user) => (
          <li
            key={user.id}
            id={`${listboxId}-${user.id}`}
            role="option"
            aria-selected={user.id === current?.id}
            aria-disabled={adding}
            className={cn(
              'cursor-pointer rounded-md px-2 py-1 aria-disabled:cursor-wait aria-disabled:opacity-50',
              user.id === current?.id && 'bg-muted',
            )}
            // Mouse down, not click: the input keeps focus (and its keyboard handling).
            onMouseDown={(event) => {
              event.preventDefault()
              void add(user)
            }}
          >
            {user.username}
            <span className="text-muted-foreground"> · {user.display_name}</span>
          </li>
        ))}
      </ul>
      <p role="status" className="text-xs text-muted-foreground">
        {term.length === 0 || candidates.data === undefined
          ? null
          : shown.length === 0
            ? t('groups.members.noCandidates')
            : t('groups.members.found', { count: shown.length })}
      </p>
      {error === undefined ? null : <ErrorAlert error={error} />}
    </div>
  )
}
