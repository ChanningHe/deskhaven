import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useUpdateUser } from '@/api/generated/users/users'
import type { RoleView, UserPatch, UserView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useMe } from '@/features/auth/session'

import { invalidateAccounts } from '../invalidate'
import { assignableRoles } from '../user-rules'

interface Draft {
  display_name: string
  email: string
  role: RoleView
  note: string
}

function draftOf(user: UserView): Draft {
  return {
    display_name: user.display_name,
    email: user.email ?? '',
    role: user.role,
    note: user.note,
  }
}

/** Only the fields that changed (`email: ""` removes the address). */
function patchOf(user: UserView, draft: Draft): UserPatch {
  const before = draftOf(user)
  return {
    ...(draft.display_name === before.display_name ? {} : { display_name: draft.display_name }),
    // Surrounding blanks are no change (a field of spaces stays as it was).
    ...(draft.email.trim() === before.email ? {} : { email: draft.email.trim() }),
    ...(draft.role === before.role ? {} : { role: draft.role }),
    ...(draft.note === before.note ? {} : { note: draft.note }),
  }
}

interface UserEditFormProps {
  user: UserView
  /** False: the fields are shown read-only (an admin looking at an owner). */
  editable: boolean
}

/** Profile and role of an account, saved in one request. */
export function UserEditForm({ user, editable }: UserEditFormProps) {
  const { t } = useTranslation()
  const me = useMe()
  const id = useId()
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState(() => draftOf(user))
  const mutation = useUpdateUser()
  const patch = patchOf(user, draft)
  const dirty = Object.keys(patch).length > 0
  // The current role stays listed even when this admin may not grant it.
  const roles = [...new Set([...assignableRoles(me), user.role])]
  const set = (change: Partial<Draft>) => {
    setDraft((previous) => ({ ...previous, ...change }))
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const updated = await mutation.mutateAsync({ id: user.id, data: patch }).catch(() => undefined)
    if (updated === undefined) return
    setDraft(draftOf(updated))
    void invalidateAccounts(queryClient)
  }

  return (
    <form className="grid gap-2" onSubmit={(event) => void submit(event)}>
      <Label htmlFor={`${id}-display`}>{t('users.fields.displayName')}</Label>
      <Input
        id={`${id}-display`}
        disabled={!editable}
        value={draft.display_name}
        onChange={(e) => {
          set({ display_name: e.target.value })
        }}
      />
      <Label htmlFor={`${id}-email`}>{t('users.fields.email')}</Label>
      <Input
        id={`${id}-email`}
        type="email"
        disabled={!editable}
        value={draft.email}
        onChange={(e) => {
          set({ email: e.target.value })
        }}
      />
      <Label htmlFor={`${id}-role`}>{t('users.fields.role')}</Label>
      <NativeSelect
        id={`${id}-role`}
        disabled={!editable}
        value={draft.role}
        onChange={(e) => {
          set({ role: e.target.value as RoleView })
        }}
      >
        {roles.map((role) => (
          <option key={role} value={role}>
            {t(`roles.${role}`)}
          </option>
        ))}
      </NativeSelect>
      <Label htmlFor={`${id}-note`}>{t('users.fields.note')}</Label>
      <textarea
        id={`${id}-note`}
        disabled={!editable}
        rows={2}
        value={draft.note}
        onChange={(e) => {
          set({ note: e.target.value })
        }}
        className="w-full rounded-lg border border-input bg-transparent px-2.5 py-1.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50"
      />
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
      {editable ? (
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm" disabled={!dirty || mutation.isPending}>
            {t('users.drawer.save')}
          </Button>
          <p role="status" className="text-xs text-muted-foreground">
            {mutation.isSuccess && !dirty ? t('users.drawer.saved') : null}
          </p>
        </div>
      ) : null}
    </form>
  )
}
