import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useListGroups } from '@/api/generated/groups/groups'
import { useAssignStrategy } from '@/api/generated/strategies/strategies'
import type { TargetBody, TargetKind } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { UserCombobox } from '@/components/common/user-combobox'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { useDeviceGroupOptions } from '@/features/devices/use-device-group-options'

import { invalidateStrategies } from '../invalidate'
import { DevicePicker } from './device-picker'

const KINDS: TargetKind[] = ['global', 'group', 'user', 'device_group', 'device']
/** Most user groups offered in the select (the API's page size limit). */
const MAX_GROUPS = 200

interface Picked {
  id: number
  label?: string
}

/** Assigns the strategy to one more target (replacing the strategy that target had). */
export function AddAssignment({ strategyId }: { strategyId: number }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [kind, setKind] = useState<TargetKind>('global')
  const [picked, setPicked] = useState<Picked | undefined>(undefined)
  const mutation = useAssignStrategy()
  const ready = kind === 'global' || picked !== undefined

  const assign = async () => {
    const data: TargetBody =
      kind === 'global' || picked === undefined
        ? { target_type: kind }
        : { target_type: kind, target_id: picked.id }
    const done = await mutation.mutateAsync({ id: strategyId, data }).then(
      () => true,
      () => false,
    )
    if (!done) return
    setPicked(undefined)
    void invalidateStrategies(queryClient)
  }

  return (
    <div className="grid gap-2 rounded-lg border p-3">
      <h3 className="text-sm font-medium">{t('strategies.assignments.add')}</h3>
      <div className="flex flex-wrap items-end gap-2">
        <div className="grid gap-1">
          <Label htmlFor={`${id}-kind`}>{t('strategies.assignments.kind')}</Label>
          <NativeSelect
            id={`${id}-kind`}
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as TargetKind)
              setPicked(undefined)
            }}
          >
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`strategies.targets.${k}`)}
              </option>
            ))}
          </NativeSelect>
        </div>
        <TargetPicker kind={kind} picked={picked} onPick={setPicked} />
        <Button
          type="button"
          size="sm"
          disabled={!ready || mutation.isPending}
          onClick={() => void assign()}
        >
          {t('strategies.assignments.assign')}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t('strategies.assignments.replaces')}</p>
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
    </div>
  )
}

interface TargetPickerProps {
  kind: TargetKind
  picked: Picked | undefined
  onPick: (picked: Picked | undefined) => void
}

function TargetPicker({ kind, picked, onPick }: TargetPickerProps) {
  const { t } = useTranslation()
  switch (kind) {
    case 'global':
      return null
    case 'device':
      return (
        <DevicePicker
          value={picked?.id}
          onChange={(id) => {
            onPick(id === undefined ? undefined : { id })
          }}
        />
      )
    case 'user':
      return picked === undefined ? (
        <UserCombobox
          label={t('strategies.targets.user')}
          listLabel={t('strategies.assignments.userCandidates')}
          foundText={(count) => t('strategies.assignments.usersFound', { count })}
          noMatchText={t('strategies.assignments.noUser')}
          onPick={(user) => {
            onPick({ id: user.id, label: user.username })
            return undefined
          }}
        />
      ) : (
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            onPick(undefined)
          }}
        >
          {t('strategies.assignments.clearUser', { name: picked.label })}
        </Button>
      )
    case 'group':
    case 'device_group':
      return <GroupSelect kind={kind} value={picked?.id} onPick={onPick} />
  }
}

interface GroupSelectProps {
  kind: 'group' | 'device_group'
  value: number | undefined
  onPick: (picked: Picked | undefined) => void
}

function GroupSelect({ kind, value, onPick }: GroupSelectProps) {
  const { t } = useTranslation()
  const id = useId()
  const userGroups = useListGroups(
    { page_size: MAX_GROUPS, sort: 'name' },
    { query: { enabled: kind === 'group' } },
  )
  const deviceGroups = useDeviceGroupOptions()
  const groups = kind === 'group' ? (userGroups.data?.items ?? []) : deviceGroups
  return (
    <div className="grid gap-1">
      <Label htmlFor={`${id}-group`}>{t(`strategies.targets.${kind}`)}</Label>
      <NativeSelect
        id={`${id}-group`}
        value={value === undefined ? '' : String(value)}
        onChange={(e) => {
          onPick(e.target.value === '' ? undefined : { id: Number(e.target.value) })
        }}
      >
        <option value="">{t('strategies.assignments.choose')}</option>
        {groups.map((group) => (
          <option key={group.id} value={group.id}>
            {group.name}
          </option>
        ))}
      </NativeSelect>
    </div>
  )
}
