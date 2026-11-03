import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { addGroupMember } from '@/api/generated/groups/groups'
import type { UserView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { UserCombobox } from '@/components/common/user-combobox'
import { invalidateAccounts } from '@/features/users/invalidate'

/**
 * "Add a member": a combobox searching the accounts outside the group (the server leaves
 * the members out, whatever page of them the drawer shows).
 */
export function AddMember({ groupId }: { groupId: number }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [adding, setAdding] = useState(false)
  const [error, setError] = useState<unknown>(undefined)

  /** `false` keeps the typed text: the user was not added. */
  const add = async (user: UserView) => {
    setAdding(true)
    setError(undefined)
    try {
      await addGroupMember(groupId, { user_id: user.id })
      await invalidateAccounts(queryClient)
      return true
    } catch (caught) {
      setError(caught)
      return false
    } finally {
      setAdding(false)
    }
  }

  return (
    <div className="grid gap-1">
      <UserCombobox
        label={t('groups.members.addSearch')}
        listLabel={t('groups.members.candidates')}
        filter={{ not_group: groupId }}
        busy={adding}
        foundText={(count) => t('groups.members.found', { count })}
        noMatchText={t('groups.members.noCandidates')}
        onPick={add}
      />
      {error === undefined ? null : <ErrorAlert error={error} />}
    </div>
  )
}
