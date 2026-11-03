import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useUpdateDevice } from '@/api/generated/devices/devices'
import type { DevicePatch, DeviceView, OwnerRef } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'

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

/** Only the fields that changed (`null` removes the owner or the group). */
function patchOf(device: DeviceView, draft: Draft): DevicePatch {
  const before = draftOf(device)
  return {
    ...(draft.owner?.id === before.owner?.id ? {} : { owner_user_id: draft.owner?.id ?? null }),
    ...(draft.groupId === before.groupId ? {} : { device_group_id: draft.groupId }),
    ...(draft.note === before.note ? {} : { note: draft.note }),
  }
}

/** Owner, device group and note of a device, saved in one request. */
export function DeviceManageForm({ device }: { device: DeviceView }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const groups = useDeviceGroupOptions()
  const [draft, setDraft] = useState(() => draftOf(device))
  const mutation = useUpdateDevice()
  const patch = patchOf(device, draft)
  const dirty = Object.keys(patch).length > 0
  const set = (change: Partial<Draft>) => {
    setDraft((previous) => ({ ...previous, ...change }))
  }
  // The current group stays listed even if the options do not include it.
  const current = device.device_group
  const options =
    groups.some((g) => g.id === current?.id) || !current ? groups : [...groups, current]

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const saved = await mutation.mutateAsync({ id: device.id, data: patch }).catch(() => undefined)
    if (saved === undefined) return
    setDraft(draftOf(saved))
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
