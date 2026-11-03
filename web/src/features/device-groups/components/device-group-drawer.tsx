import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteDeviceGroup,
  getGetDeviceGroupQueryKey,
  useGetDeviceGroup,
  useUpdateDeviceGroup,
} from '@/api/generated/device-groups/device-groups'
import type { DeviceGroupView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'
import { Button, buttonVariants } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { invalidateDevices } from '@/features/devices/invalidate'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

interface DeviceGroupDrawerProps {
  groupId: number | undefined
  onClose: () => void
}

/** Side drawer of one device group: name and note, its devices, delete. */
export function DeviceGroupDrawer({ groupId, onClose }: DeviceGroupDrawerProps) {
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
  const query = useGetDeviceGroup(groupId)
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('deviceGroups.drawer.title')}</SheetTitle>
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
          {t('deviceGroups.drawer.devices', { count: group.device_count })}
        </SheetDescription>
        {editable ? null : (
          <div className="flex items-center gap-2 pt-2 text-xs text-muted-foreground">
            <SourceBadge source={{ kind: 'file' }} />
            {t('deviceGroups.drawer.readOnly')}
          </div>
        )}
      </SheetHeader>
      <div className="grid gap-6 px-4 pb-6">
        {editable ? <GroupForm key={group.id} group={group} onDeleted={onDeleted} /> : null}
        <Link
          to="/devices"
          search={{ device_group: group.id }}
          className={buttonVariants({ variant: 'outline', size: 'sm', className: 'w-fit' })}
        >
          {t('deviceGroups.drawer.viewDevices')}
        </Link>
      </div>
    </>
  )
}

function GroupForm({ group, onDeleted }: { group: DeviceGroupView; onDeleted: () => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const { draft, changed, conflicts, set, reset } = useRebasedDraft({
    name: group.name,
    note: group.note,
  })
  const mutation = useUpdateDeviceGroup()
  const dirty = changed.length > 0
  const detailKey = getGetDeviceGroupQueryKey(group.id)

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
    queryClient.setQueryData<DeviceGroupView>(detailKey, saved)
    reset({ name: saved.name, note: saved.note })
    void invalidateDevices(queryClient)
  }

  // The confirm dialog is a sibling of the form, never inside it: React bubbles its submit
  // through the portal, which would also save the form.
  return (
    <>
      <form className="grid gap-2" onSubmit={(event) => void submit(event)}>
        <Label htmlFor={`${id}-name`}>{t('deviceGroups.fields.name')}</Label>
        <Input
          id={`${id}-name`}
          required
          maxLength={64}
          value={draft.name}
          onChange={(e) => {
            set({ name: e.target.value })
          }}
        />
        <Label htmlFor={`${id}-note`}>{t('deviceGroups.fields.note')}</Label>
        <Input
          id={`${id}-note`}
          value={draft.note}
          onChange={(e) => {
            set({ note: e.target.value })
          }}
        />
        {conflicts.length > 0 ? (
          <p role="status" className="text-xs text-warning">
            {t('deviceGroups.drawer.conflict')}
          </p>
        ) : null}
        {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
        <div className="flex gap-2">
          <Button type="submit" size="sm" disabled={!dirty || mutation.isPending}>
            {t('deviceGroups.drawer.save')}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="destructive"
            onClick={() => {
              setConfirming(true)
            }}
          >
            {t('deviceGroups.drawer.delete')}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('deviceGroups.confirmDelete.title')}
        description={t('deviceGroups.confirmDelete.description', { name: group.name })}
        confirmLabel={t('deviceGroups.confirmDelete.confirm')}
        confirmText={group.name}
        onConfirm={async () => {
          await deleteDeviceGroup(group.id)
          onDeleted()
          // Nothing to refetch: the group is gone.
          queryClient.removeQueries({ queryKey: detailKey })
          await invalidateDevices(queryClient)
        }}
      />
    </>
  )
}
