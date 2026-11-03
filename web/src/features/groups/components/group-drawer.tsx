import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteGroup,
  getGetGroupQueryKey,
  useGetGroup,
  useUpdateGroup,
} from '@/api/generated/groups/groups'
import type { GroupView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { invalidateAccounts } from '@/features/users/invalidate'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { GroupMembers } from './group-members'

interface GroupDrawerProps {
  groupId: number | undefined
  onClose: () => void
}

/** Side drawer of one group: name and note, members, delete. */
export function GroupDrawer({ groupId, onClose }: GroupDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={groupId !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-md">
        {groupId === undefined ? null : <DrawerBody groupId={groupId} onDeleted={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({ groupId, onDeleted }: { groupId: number; onDeleted: () => void }) {
  const { t } = useTranslation()
  const query = useGetGroup(groupId)
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('groups.drawer.title')}</SheetTitle>
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <SheetDescription>{t('common.loading')}</SheetDescription>
        )}
      </SheetHeader>
    )
  }
  const group = query.data
  const editable = group.managed_by === 'ui'
  return (
    <>
      <SheetHeader>
        <SheetTitle>{group.name}</SheetTitle>
        <SheetDescription>
          {t('groups.drawer.members', { count: group.member_count })}
        </SheetDescription>
        {editable ? null : (
          <div className="flex items-center gap-2 pt-2 text-xs text-muted-foreground">
            <SourceBadge source={{ kind: 'file' }} />
            {t('groups.drawer.readOnly')}
          </div>
        )}
      </SheetHeader>
      <div className="grid gap-6 px-4 pb-6">
        {editable ? <GroupForm key={group.id} group={group} onDeleted={onDeleted} /> : null}
        <GroupMembers groupId={group.id} editable={editable} />
      </div>
    </>
  )
}

function GroupForm({ group, onDeleted }: { group: GroupView; onDeleted: () => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const { draft, changed, conflicts, set, reset } = useRebasedDraft({
    name: group.name,
    note: group.note,
  })
  const mutation = useUpdateGroup()
  const dirty = changed.length > 0
  const detailKey = getGetGroupQueryKey(group.id)

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    // Only what the user edited: an unrelated field must not write a stale value back.
    const data = {
      ...(changed.includes('name') ? { name: draft.name } : {}),
      ...(changed.includes('note') ? { note: draft.note } : {}),
    }
    const saved = await mutation.mutateAsync({ id: group.id, data }).catch(() => undefined)
    if (saved === undefined) return
    // The response is the newest copy: a refetch still in flight must not overwrite it.
    await queryClient.cancelQueries({ queryKey: detailKey })
    queryClient.setQueryData<GroupView>(detailKey, saved)
    reset({ name: saved.name, note: saved.note }, data)
    void invalidateAccounts(queryClient)
  }

  // The confirm dialog is a sibling of the form, never inside it: React bubbles its submit
  // through the portal, which would also save the form.
  return (
    <>
      <form className="grid gap-2" onSubmit={(event) => void submit(event)}>
        <Label htmlFor={`${id}-name`}>{t('groups.fields.name')}</Label>
        <Input
          id={`${id}-name`}
          required
          maxLength={64}
          value={draft.name}
          onChange={(e) => {
            set({ name: e.target.value })
          }}
        />
        <Label htmlFor={`${id}-note`}>{t('groups.fields.note')}</Label>
        <Input
          id={`${id}-note`}
          value={draft.note}
          onChange={(e) => {
            set({ note: e.target.value })
          }}
        />
        {conflicts.length > 0 ? (
          <p role="status" className="text-xs text-warning">
            {t('groups.drawer.conflict')}
          </p>
        ) : null}
        {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
        <div className="flex gap-2">
          <Button type="submit" size="sm" disabled={!dirty || mutation.isPending}>
            {t('groups.drawer.save')}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="destructive"
            onClick={() => {
              setConfirming(true)
            }}
          >
            {t('groups.drawer.delete')}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('groups.confirmDelete.title')}
        description={t('groups.confirmDelete.description', { name: group.name })}
        confirmLabel={t('groups.confirmDelete.confirm')}
        confirmText={group.name}
        onConfirm={async () => {
          await deleteGroup(group.id)
          onDeleted()
          await invalidateAccounts(queryClient)
        }}
      />
    </>
  )
}
