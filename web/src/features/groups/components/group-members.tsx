import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  addGroupMember,
  removeGroupMember,
  useListGroupMembers,
} from '@/api/generated/groups/groups'
import { useListUsers } from '@/api/generated/users/users'
import type { UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { invalidateAccounts } from '@/features/users/invalidate'

/** Members shown at once; the user list (filter by group) has them all. */
const MEMBER_PAGE = 50
const CANDIDATES = 8

interface GroupMembersProps {
  groupId: number
  /** False for groups managed by the configuration file. */
  editable: boolean
}

export function GroupMembers({ groupId, editable }: GroupMembersProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const members = useListGroupMembers(groupId, { page_size: MEMBER_PAGE, sort: 'username' })
  const [removing, setRemoving] = useState<UserView | null>(null)
  const [error, setError] = useState<unknown>(undefined)

  const add = async (user: UserView) => {
    setError(undefined)
    try {
      await addGroupMember(groupId, { user_id: user.id })
      await invalidateAccounts(queryClient)
    } catch (caught) {
      setError(caught)
    }
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium">{t('groups.members.title')}</h3>
      {members.error ? <ErrorAlert error={members.error} /> : null}
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
      {editable ? (
        <AddMember memberIds={members.data?.items.map((u) => u.id) ?? []} onAdd={add} />
      ) : null}
      {error === undefined ? null : <ErrorAlert error={error} />}
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null)
        }}
        title={t('groups.members.confirmTitle')}
        description={t('groups.members.confirmDescription', { username: removing?.username ?? '' })}
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

function AddMember({
  memberIds,
  onAdd,
}: {
  memberIds: number[]
  onAdd: (user: UserView) => Promise<void>
}) {
  const { t } = useTranslation()
  const [q, setQ] = useState('')
  const term = q.trim()
  const candidates = useListUsers(
    { q: term, page_size: CANDIDATES },
    { query: { enabled: term.length > 0 } },
  )
  const shown = (candidates.data?.items ?? []).filter((user) => !memberIds.includes(user.id))
  return (
    <div className="grid gap-1">
      <Input
        aria-label={t('groups.members.addSearch')}
        placeholder={t('groups.members.addSearch')}
        value={q}
        onChange={(e) => {
          setQ(e.target.value)
        }}
      />
      <ul className="grid gap-1 text-sm">
        {term.length > 0 && shown.length === 0 && candidates.data !== undefined ? (
          <li className="text-xs text-muted-foreground">{t('groups.members.noCandidates')}</li>
        ) : null}
        {shown.map((user) => (
          <li key={user.id} className="flex items-center justify-between gap-2">
            <span>{user.username}</span>
            <Button
              size="xs"
              variant="outline"
              aria-label={t('groups.members.add', { username: user.username })}
              onClick={() => void onAdd(user)}
            >
              {t('groups.members.addShort')}
            </Button>
          </li>
        ))}
      </ul>
    </div>
  )
}
