import { useTranslation } from 'react-i18next'

import { NativeSelect } from '@/components/common/native-select'

import { AUTH_SOURCES, ROLES, STATUSES } from '../users-search'
import type { UsersSearch } from '../users-search'

type FilterKey = 'role' | 'status' | 'auth_source'

interface Filter {
  key: FilterKey
  label: string
  options: { value: string; label: string }[]
}

interface UserFiltersProps {
  search: UsersSearch
  onChange: (patch: Partial<UsersSearch>) => void
}

/** Role, status and source filters of the user list (URL-synced through `onChange`). */
export function UserFilters({ search, onChange }: UserFiltersProps) {
  const { t } = useTranslation()
  const filters: Filter[] = [
    {
      key: 'role',
      label: t('users.filters.role'),
      options: ROLES.map((value) => ({ value, label: t(`roles.${value}`) })),
    },
    {
      key: 'status',
      label: t('users.filters.status'),
      options: STATUSES.map((value) => ({ value, label: t(`users.statuses.${value}`) })),
    },
    {
      key: 'auth_source',
      label: t('users.filters.source'),
      options: AUTH_SOURCES.map((value) => ({ value, label: t(`users.sources.${value}`) })),
    },
  ]
  return (
    <>
      {filters.map(({ key, label, options }) => (
        <NativeSelect
          key={key}
          aria-label={label}
          value={search[key] ?? ''}
          onChange={(event) => {
            const value = event.target.value
            // One of `options` (or empty); the route's validateSearch re-checks it.
            onChange({ [key]: value === '' ? undefined : value })
          }}
        >
          <option value="">{label}</option>
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </NativeSelect>
      ))}
    </>
  )
}
