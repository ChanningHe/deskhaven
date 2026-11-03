import { useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { getGetDeviceQueryKey, useUpdateDevice } from '@/api/generated/devices/devices'
import type { DeviceDetail, DevicePatch, DeviceView, OwnerRef } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { invalidateDevices } from '../invalidate'
import { useDeviceGroupOptions } from '../use-device-group-options'
import { OwnerPicker } from './owner-picker'

interface Draft {
  owner: OwnerRef | null
  groupId: number | null
  note: string
}

function draftOf(device: DeviceView): Draft {
  return {
    owner: device.owner ?? null,
    groupId: device.device_group?.id ?? null,
    note: device.note,
  }
}

/** Only the fields the user edited (`null` removes the owner or the group). */
function patchOf(draft: Draft, changed: (keyof Draft)[]): DevicePatch {
  return {
    ...(changed.includes('owner') ? { owner_user_id: draft.owner?.id ?? null } : {}),
    ...(changed.includes('groupId') ? { device_group_id: draft.groupId } : {}),
    ...(changed.includes('note') ? { note: draft.note } : {}),
  }
}

/** Owner, device group and note of a device, saved in one request. */
export function DeviceManageForm({ device }: { device: DeviceView }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const groups = useDeviceGroupOptions()
  const { draft, changed, conflicts, set, reset } = useRebasedDraft(draftOf(device))
  const mutation = useUpdateDevice()
  const dirty = changed.length > 0
  // The current group stays listed even if the options do not include it.
  const current = device.device_group
  const options =
    groups.some((g) => g.id === current?.id) || !current ? groups : [...groups, current]

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const patch = patchOf(draft, changed)
    const saved = await mutation.mutateAsync({ id: device.id, data: patch }).catch(() => undefined)
    if (saved === undefined) return
    // The response is the newest copy: a poll still in flight must not overwrite it.
    const key = getGetDeviceQueryKey(device.id)
    await queryClient.cancelQueries({ queryKey: key })
    queryClient.setQueryData<DeviceDetail>(key, (old) =>
      old === undefined ? old : { ...old, device: saved },
    )
    reset(draftOf(saved))
    void invalidateDevices(queryClient)
  }

  return (
    <form className="grid gap-2" onSubmit={(event) => void submit(event)}>
      <h3 className="text-sm font-medium">{t('devices.manage.title')}</h3>
      <div role="group" aria-labelledby={`${id}-owner`} className="grid gap-2">
        <span id={`${id}-owner`} className="text-sm leading-none font-medium">
          {t('devices.manage.owner')}
        </span>
        <OwnerPicker
          value={draft.owner}
          onChange={(owner) => {
            set({ owner })
          }}
        />
      </div>
      <Label htmlFor={`${id}-group`}>{t('devices.manage.group')}</Label>
      <NativeSelect
        id={`${id}-group`}
        value={draft.groupId === null ? '' : String(draft.groupId)}
        onChange={(e) => {
          set({ groupId: e.target.value === '' ? null : Number(e.target.value) })
        }}
      >
        <option value="">{t('devices.manage.noGroup')}</option>
        {options.map((group) => (
          <option key={group.id} value={group.id}>
            {group.name}
          </option>
        ))}
      </NativeSelect>
      <Label htmlFor={`${id}-note`}>{t('devices.manage.note')}</Label>
      <textarea
        id={`${id}-note`}
        rows={2}
        maxLength={2000}
        value={draft.note}
        onChange={(e) => {
          set({ note: e.target.value })
        }}
        className="w-full rounded-lg border border-input bg-transparent px-2.5 py-1.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
      />
      {conflicts.length > 0 ? (
        <p role="status" className="text-xs text-warning">
          {t('devices.manage.conflict')}
        </p>
      ) : null}
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" disabled={!dirty || mutation.isPending}>
          {t('devices.drawer.save')}
        </Button>
        <p role="status" className="text-xs text-muted-foreground">
          {mutation.isSuccess && !dirty ? t('devices.drawer.saved') : null}
        </p>
      </div>
    </form>
  )
}
