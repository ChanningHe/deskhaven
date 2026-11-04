import { X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useGetUser } from '@/api/generated/users/users'
import { NativeSelect } from '@/components/common/native-select'
import { UserCombobox } from '@/components/common/user-combobox'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

import { endField, startField, windowEnd, windowStart } from '../audit-range'
import type { AuditSearch } from '../audit-search'

interface ChangeProps {
  search: AuditSearch
  onChange: (patch: Partial<AuditSearch>) => void
}

/** The first and last day of the window (the browser's time zone). */
export function WindowFilter({ search, onChange }: ChangeProps) {
  const { t } = useTranslation()
  return (
    <>
      <Input
        type="date"
        className="w-40"
        aria-label={t('audit.filters.from')}
        value={startField(search.from)}
        max={endField(search.to) || undefined}
        onChange={(event) => {
          const from = windowStart(event.target.value)
          // A start after the end would be an empty window (the API refuses it): drop the end.
          const past = from !== undefined && search.to !== undefined && from >= search.to
          onChange(past ? { from, to: undefined } : { from })
        }}
      />
      <Input
        type="date"
        className="w-40"
        aria-label={t('audit.filters.to')}
        value={endField(search.to)}
        min={startField(search.from) || undefined}
        onChange={(event) => {
          const to = windowEnd(event.target.value)
          const before = to !== undefined && search.from !== undefined && to <= search.from
          onChange(before ? { to, from: undefined } : { to })
        }}
      />
    </>
  )
}

interface TextFilterProps {
  label: string
  value: string | undefined
  onCommit: (value: string | undefined) => void
}

/** A text filter applied on Enter or when the field loses focus (not per keystroke). */
export function TextFilter({ label, value, onCommit }: TextFilterProps) {
  const [draft, setDraft] = useState(value ?? '')
  const [seen, setSeen] = useState(value)
  // The URL changed underneath (back button, "clear filters"): follow it.
  if (value !== seen) {
    setSeen(value)
    setDraft(value ?? '')
  }
  const commit = () => {
    const next = draft.trim()
    if (next !== (value ?? '')) onCommit(next === '' ? undefined : next)
  }
  return (
    <Input
      className="w-40"
      aria-label={label}
      placeholder={label}
      value={draft}
      onChange={(event) => {
        setDraft(event.target.value)
      }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === 'Enter') commit()
      }}
    />
  )
}

interface SelectFilterProps<T extends string | number> {
  label: string
  value: T | undefined
  options: readonly { value: T; label: string }[]
  onChange: (value: T | undefined) => void
}

/** A dropdown over a short fixed list; the first entry is "no filter". */
export function SelectFilter<T extends string | number>(props: SelectFilterProps<T>) {
  const { label, value, options, onChange } = props
  return (
    <NativeSelect
      aria-label={label}
      value={value === undefined ? '' : String(value)}
      onChange={(event) => {
        const picked = options.find((option) => String(option.value) === event.target.value)
        onChange(picked?.value)
      }}
    >
      <option value="">{label}</option>
      {options.map((option) => (
        <option key={String(option.value)} value={String(option.value)}>
          {option.label}
        </option>
      ))}
    </NativeSelect>
  )
}

/** The user filter: the chosen account with a way to clear it, or a search over accounts. */
export function UserFilter({ search, onChange }: ChangeProps) {
  const { t } = useTranslation()
  const chosen = search.user
  if (chosen !== undefined)
    return (
      <ChosenUser
        id={chosen}
        onClear={() => {
          onChange({ user: undefined })
        }}
      />
    )
  return (
    <div className="w-48">
      <UserCombobox
        label={t('audit.filters.user')}
        listLabel={t('audit.filters.userCandidates')}
        placeholder={t('audit.filters.user')}
        foundText={(count) => t('audit.filters.userFound', { count })}
        noMatchText={t('audit.filters.userNoMatch')}
        onPick={(user) => {
          onChange({ user: user.id })
          return true
        }}
      />
    </div>
  )
}

function ChosenUser({ id, onClear }: { id: number; onClear: () => void }) {
  const { t } = useTranslation()
  const user = useGetUser(id)
  const name = user.data?.user.username ?? `#${String(id)}`
  return (
    <div className="flex h-8 items-center gap-1 rounded-lg border px-2 text-sm">
      <span>{t('audit.filters.userIs', { name })}</span>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={t('audit.filters.userClear')}
        onClick={onClear}
      >
        <X aria-hidden />
      </Button>
    </div>
  )
}
