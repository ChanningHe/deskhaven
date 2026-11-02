import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { removeGroupMember, useListGroupMembers } from '@/api/generated/groups/groups'
import type { UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { invalidateAccounts } from '@/features/users/invalidate'

import { AddMember } from './add-member'

/** Members per page of the drawer's list. */
const MEMBER_PAGE_SIZE = 50

interface GroupMembersProps {
  groupId: number
  /** False for groups managed by the configuration file. */
  editable: boolean
}

/** A group's members, page by page, with remove (confirmed) and add (search). */
export function GroupMembers({ groupId, editable }: GroupMembersProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [page, setPage] = useState(1)
  const members = useListGroupMembers(
    groupId,
    { page, page_size: MEMBER_PAGE_SIZE, sort: 'username' },
    { query: { placeholderData: keepPreviousData } },
  )
  const [removing, setRemoving] = useState<UserView | null>(null)
  const total = members.data?.total ?? 0
  const pages = Math.max(1, Math.ceil(total / MEMBER_PAGE_SIZE))
  // A removal can empty the last page: step back to one that exists.
  if (page > pages && members.data !== undefined && !members.isPlaceholderData) setPage(pages)

  return (
    <section className="grid gap-2">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-medium">{t('groups.members.title')}</h3>
        {members.data === undefined ? null : (
          <span className="text-xs text-muted-foreground">
            {t('groups.members.total', { count: total })}
          </span>
        )}
      </div>
      {members.error ? (
        <ErrorAlert error={members.error} onRetry={() => void members.refetch()} />
      ) : null}
      {members.data?.items.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('groups.members.empty')}</p>
      ) : null}
      <ul className="grid gap-1 text-sm">
        {members.data?.items.map((user) => (
          <li key={user.id} className="flex items-center justify-between gap-2">
            <span>
              {user.username}
              <span className="text-muted-foreground"> · {user.display_name}</span>
            </span>
            {editable ? (
              <Button
                size="xs"
                variant="ghost"
                aria-label={t('groups.members.remove', { username: user.username })}
                onClick={() => {
                  setRemoving(user)
                }}
              >
                {t('groups.members.removeShort')}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      {pages > 1 ? (
        <nav
          aria-label={t('groups.members.pagination')}
          className="flex items-center justify-between gap-2 text-xs text-muted-foreground"
        >
          <Button
            size="xs"
            variant="outline"
            disabled={page <= 1}
            onClick={() => {
              setPage(page - 1)
            }}
          >
            {t('dataTable.previousPage')}
          </Button>
          <span>{t('groups.members.page', { page, pages })}</span>
          <Button
            size="xs"
            variant="outline"
            disabled={page >= pages}
            onClick={() => {
              setPage(page + 1)
            }}
          >
            {t('dataTable.nextPage')}
          </Button>
        </nav>
      ) : null}
      {editable ? <AddMember groupId={groupId} /> : null}
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null)
        }}
        title={t('groups.members.confirmTitle')}
        description={t('groups.members.confirmDescription', {
          username: removing?.username ?? '',
        })}
        confirmLabel={t('groups.members.removeShort')}
        onConfirm={async () => {
          if (removing === null) return
          await removeGroupMember(groupId, removing.id)
          await invalidateAccounts(queryClient)
        }}
      />
    </section>
  )
}
