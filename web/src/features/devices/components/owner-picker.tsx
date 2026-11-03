import { useId, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useListUsers } from '@/api/generated/users/users'
import type { OwnerRef, UserView } from '@/api/generated/model'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useDebouncedValue } from '@/lib/use-debounced-value'
import { cn } from '@/lib/utils'

/** Candidates offered at once; typing more narrows them. */
const CANDIDATES = 8

interface OwnerPickerProps {
  value: OwnerRef | null
  onChange: (owner: OwnerRef | null) => void
  disabled?: boolean
}

/**
 * The owner of a device: the chosen user with a way to remove them, or a combobox searching
 * the accounts. Arrow keys move through the suggestions, Enter or a click picks one.
 */
export function OwnerPicker({ value, onChange, disabled = false }: OwnerPickerProps) {
  const { t } = useTranslation()
  if (value !== null) {
    return (
      <div className="flex items-center justify-between gap-2 text-sm">
        <span>{value.username}</span>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => {
            onChange(null)
          }}
        >
          {t('devices.manage.ownerClear')}
        </Button>
      </div>
    )
  }
  return <OwnerSearch onPick={onChange} disabled={disabled} />
}

function OwnerSearch({
  onPick,
  disabled,
}: {
  onPick: (owner: OwnerRef) => void
  disabled: boolean
}) {
  const { t } = useTranslation()
  const listboxId = useId()
  const [q, setQ] = useState('')
  const [active, setActive] = useState(0)
  const term = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const candidates = useListUsers(
    { q: term, page_size: CANDIDATES, sort: 'username' },
    { query: { enabled: term.length > 0 } },
  )
  const shown = term.length > 0 ? (candidates.data?.items ?? []) : []
  const expanded = shown.length > 0
  const current = shown[Math.min(active, shown.length - 1)]

  const pick = (user: UserView) => {
    onPick({ id: user.id, username: user.username })
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (!expanded) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      setActive((index) => (index + step + shown.length) % shown.length)
    } else if (event.key === 'Enter' && current !== undefined) {
      // The drawer's form must not submit.
      event.preventDefault()
      pick(current)
    } else if (event.key === 'Escape') {
      event.preventDefault()
      setQ('')
    }
  }

  return (
    <div className="grid gap-1">
      <Input
        role="combobox"
        aria-label={t('devices.manage.ownerSearch')}
        aria-expanded={expanded}
        aria-controls={listboxId}
        aria-autocomplete="list"
        aria-activedescendant={current ? `${listboxId}-${current.id}` : undefined}
        disabled={disabled}
        placeholder={t('devices.manage.ownerNone')}
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
        aria-label={t('devices.manage.ownerCandidates')}
        hidden={!expanded}
        className="grid gap-0.5 rounded-lg border p-1 text-sm"
      >
        {shown.map((user) => (
          <li
            key={user.id}
            id={`${listboxId}-${user.id}`}
            role="option"
            aria-selected={user.id === current?.id}
            className={cn(
              'cursor-pointer rounded-md px-2 py-1',
              user.id === current?.id && 'bg-muted',
            )}
            // Mouse down, not click: the input keeps focus (and its keyboard handling).
            onMouseDown={(event) => {
              event.preventDefault()
              pick(user)
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
            ? t('devices.manage.ownerNoMatch')
            : t('devices.manage.ownerFound', { count: shown.length })}
      </p>
    </div>
  )
}
