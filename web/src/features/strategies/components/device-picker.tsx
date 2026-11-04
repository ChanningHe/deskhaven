import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useListDevices } from '@/api/generated/devices/devices'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
import { NativeSelect } from '@/components/common/native-select'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useDebouncedValue } from '@/lib/use-debounced-value'

/** Devices offered at once; typing narrows them. */
const CANDIDATES = 20

interface DevicePickerProps {
  value: number | undefined
  onChange: (id: number | undefined) => void
}

/** A device search (id, name) with a select of the matches. */
export function DevicePicker({ value, onChange }: DevicePickerProps) {
  const { t } = useTranslation()
  const id = useId()
  const [q, setQ] = useState('')
  const debounced = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const query = useListDevices({
    page_size: CANDIDATES,
    ...(debounced === '' ? {} : { q: debounced }),
  })
  const devices = query.data?.items ?? []
  return (
    <div className="flex flex-wrap items-end gap-2">
      <div className="grid gap-1">
        <Label htmlFor={`${id}-q`}>{t('strategies.devicePicker.search')}</Label>
        <Input
          id={`${id}-q`}
          type="search"
          value={q}
          onChange={(e) => {
            setQ(e.target.value)
          }}
        />
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`${id}-device`}>{t('strategies.devicePicker.device')}</Label>
        <NativeSelect
          id={`${id}-device`}
          value={value === undefined ? '' : String(value)}
          onChange={(e) => {
            onChange(e.target.value === '' ? undefined : Number(e.target.value))
          }}
        >
          <option value="">{t('strategies.devicePicker.choose')}</option>
          {devices.map((device) => (
            <option key={device.id} value={device.id}>
              {device.hostname == null
                ? device.rustdesk_id
                : t('strategies.devicePicker.option', {
                    id: device.rustdesk_id,
                    name: device.hostname,
                  })}
            </option>
          ))}
        </NativeSelect>
      </div>
    </div>
  )
}
