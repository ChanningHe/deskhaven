import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteDeviceGroup,
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
  const [name, setName] = useState(group.name)
  const [note, setNote] = useState(group.note)
  const [confirming, setConfirming] = useState(false)
  const mutation = useUpdateDeviceGroup()
  const dirty = name !== group.name || note !== group.note

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const data = {
      ...(name === group.name ? {} : { name }),
      ...(note === group.note ? {} : { note }),
    }
    const saved = await mutation.mutateAsync({ id: group.id, data }).catch(() => undefined)
    if (saved !== undefined) void invalidateDevices(queryClient)
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
          value={name}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
        <Label htmlFor={`${id}-note`}>{t('deviceGroups.fields.note')}</Label>
        <Input
          id={`${id}-note`}
          value={note}
          onChange={(e) => {
            setNote(e.target.value)
          }}
        />
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
          await invalidateDevices(queryClient)
        }}
      />
    </>
  )
}
