import { keepPreviousData } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Users } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListUsers } from '@/api/generated/users/users'
import { applyTableSearch, DataTable, toListParams } from '@/components/common/data-table'
import type { SearchChangeOptions } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'

import { hasUserFilters, USER_SORTABLE, usersListParams } from '../users-search'
import type { UsersSearch } from '../users-search'
import { CreateUserButton } from './create-user-dialog'
import { ImportUsersButton } from './import-users-dialog'
import { useResetPasswordReveal } from './reset-password-reveal'
import { userColumns } from './user-columns'
import { UserDrawer } from './user-drawer'
import { UserFilters } from './user-filters'

export function UsersPage() {
  const { t, i18n } = useTranslation()
  const search = useSearch({ from: '/_app/users' })
  const navigate = useNavigate({ from: '/users' })
  // The previous page stays on screen while the next one loads (no flash of the skeleton).
  const query = useListUsers(usersListParams(search, toListParams(search)), {
    query: { placeholderData: keepPreviousData },
  })
  const { resetPassword, dialog: resetReveal } = useResetPasswordReveal()

  const onSearchChange = useCallback(
    (patch: Partial<UsersSearch>, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: UsersSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  // The drawer is not a filter: opening it keeps the page. Opening adds a history entry;
  // closing (Escape, close button, after a delete) replaces it, so Back never reopens it.
  const openUser = useCallback(
    (user: number) => {
      void navigate({ search: (previous: UsersSearch) => ({ ...previous, user }) })
    },
    [navigate],
  )
  const closeUser = useCallback(() => {
    void navigate({
      search: (previous: UsersSearch) => ({ ...previous, user: undefined }),
      replace: true,
    })
  }, [navigate])
  const columns = useMemo(
    () => userColumns(t, i18n.language, openUser),
    [t, i18n.language, openUser],
  )

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('nav.users')}
        description={t('pages.users.description')}
        actions={
          <>
            <ImportUsersButton />
            <CreateUserButton />
          </>
        }
      />
      <DataTable
        id="users"
        label={t('nav.users')}
        columns={columns}
        sortable={USER_SORTABLE}
        data={query.data}
        getRowId={(user) => String(user.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('users.search')}
        filters={<UserFilters search={search} onChange={onSearchChange} />}
        isFiltered={hasUserFilters(search)}
        onClearFilters={() => {
          onSearchChange({
            q: undefined,
            role: undefined,
            status: undefined,
            auth_source: undefined,
          })
        }}
        emptyState={
          <EmptyState
            icon={Users}
            title={t('pages.users.emptyTitle')}
            description={t('pages.users.emptyDescription')}
          />
        }
      />
      <UserDrawer userId={search.user} onClose={closeUser} onResetPassword={resetPassword} />
      {resetReveal}
    </div>
  )
}
