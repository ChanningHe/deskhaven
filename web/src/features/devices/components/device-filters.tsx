import { useTranslation } from 'react-i18next'

import { NativeSelect } from '@/components/common/native-select'

import { DEVICE_STATUSES } from '../devices-search'
import type { DevicesSearch } from '../devices-search'
import { useDeviceGroupOptions } from '../use-device-group-options'

interface DeviceFiltersProps {
  search: DevicesSearch
  onChange: (patch: Partial<DevicesSearch>) => void
}

const ONLINE_VALUES = { true: true, false: false } as const

/** Online status, device group and status filters of the device list (URL-synced). */
export function DeviceFilters({ search, onChange }: DeviceFiltersProps) {
  const { t } = useTranslation()
  const groups = useDeviceGroupOptions()
  return (
    <>
      <NativeSelect
        aria-label={t('devices.filters.online')}
        value={search.online === undefined ? '' : String(search.online)}
        onChange={(event) => {
          const value = event.target.value
          onChange({ online: value === '' ? undefined : ONLINE_VALUES[value as 'true' | 'false'] })
        }}
      >
        <option value="">{t('devices.filters.online')}</option>
        <option value="true">{t('devices.filters.onlineYes')}</option>
        <option value="false">{t('devices.filters.onlineNo')}</option>
      </NativeSelect>
      <NativeSelect
        aria-label={t('devices.filters.group')}
        value={search.device_group === undefined ? '' : String(search.device_group)}
        onChange={(event) => {
          const value = event.target.value
          onChange({ device_group: value === '' ? undefined : Number(value) })
        }}
      >
        <option value="">{t('devices.filters.group')}</option>
        {groups.map((group) => (
          <option key={group.id} value={group.id}>
            {group.name}
          </option>
        ))}
      </NativeSelect>
      <NativeSelect
        aria-label={t('devices.filters.status')}
        value={search.status ?? ''}
        onChange={(event) => {
          const value = event.target.value
          // One of DEVICE_STATUSES (or empty); the route's validateSearch re-checks it.
          onChange({ status: DEVICE_STATUSES.find((status) => status === value) })
        }}
      >
        <option value="">{t('devices.filters.status')}</option>
        <option value="active">{t('devices.filters.statusActive')}</option>
        <option value="disabled">{t('devices.filters.statusDisabled')}</option>
      </NativeSelect>
    </>
  )
}
