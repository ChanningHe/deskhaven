import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useId } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteDeviceGroup,
  getGetDeviceGroupQueryKey,
  useGetDeviceGroup,
  useUpdateDeviceGroup,
} from '@/api/generated/device-groups/device-groups'
import type { DeviceGroupView } from '@/api/generated/model'
import { DangerZone } from '@/components/common/danger-zone'
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
import { httpStatusOf } from '@/lib/http-error'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

const HTTP_NOT_FOUND = 404

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
  // A later refetch failed: what follows is the last good copy (or the group is gone).
  const failure = query.isError ? query.error : undefined
  const gone = httpStatusOf(failure) === HTTP_NOT_FOUND
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
      {gone ? (
        <p role="alert" className="mx-4 rounded-lg bg-muted px-3 py-2 text-sm">
          {t('deviceGroups.drawer.gone')}
        </p>
      ) : (
        <div className="grid gap-6 px-4 pb-6">
          {failure === undefined ? null : (
            <ErrorAlert
              error={failure}
              title={t('deviceGroups.drawer.stale')}
              onRetry={() => void query.refetch()}
            />
          )}
          {editable ? <GroupForm key={group.id} group={group} /> : null}
          <Link
            to="/devices"
            search={{ device_group: group.id }}
            className={buttonVariants({ variant: 'outline', size: 'sm', className: 'w-fit' })}
          >
            {t('deviceGroups.drawer.viewDevices')}
          </Link>
          {editable ? <GroupDangerZone group={group} onDeleted={onDeleted} /> : null}
        </div>
      )}
    </>
  )
}

function GroupForm({ group }: { group: DeviceGroupView }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
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
    const sent = data
    const saved = await mutation.mutateAsync({ id: group.id, data }).catch(() => undefined)
    if (saved === undefined) return
    // The response is the newest copy: a refetch still in flight must not overwrite it.
    await queryClient.cancelQueries({ queryKey: detailKey })
    queryClient.setQueryData<DeviceGroupView>(detailKey, saved)
    reset({ name: saved.name, note: saved.note }, sent)
    void invalidateDevices(queryClient)
  }

  return (
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
      <Button type="submit" size="sm" className="w-fit" disabled={!dirty || mutation.isPending}>
        {t('deviceGroups.drawer.save')}
      </Button>
    </form>
  )
}

function GroupDangerZone({ group, onDeleted }: { group: DeviceGroupView; onDeleted: () => void }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  return (
    <DangerZone
      label={t('deviceGroups.drawer.delete')}
      title={t('deviceGroups.confirmDelete.title')}
      description={t('deviceGroups.confirmDelete.description', { name: group.name })}
      confirmLabel={t('deviceGroups.confirmDelete.confirm')}
      confirmText={group.name}
      onConfirm={async () => {
        await deleteDeviceGroup(group.id)
        onDeleted()
        // Nothing to refetch: the group is gone.
        queryClient.removeQueries({ queryKey: getGetDeviceGroupQueryKey(group.id) })
        await invalidateDevices(queryClient)
      }}
    />
  )
}
