import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import type { OwnerRef } from '@/api/generated/model'
import { UserCombobox } from '@/components/common/user-combobox'
import { Button } from '@/components/ui/button'

interface OwnerPickerProps {
  value: OwnerRef | null
  onChange: (owner: OwnerRef | null) => void
  disabled?: boolean
}

/**
 * The owner of a device: the chosen user with a way to remove them, or a combobox searching
 * the accounts. Focus follows the change: removing the owner focuses the search field,
 * picking one focuses the remove button.
 */
export function OwnerPicker({ value, onChange, disabled = false }: OwnerPickerProps) {
  const { t } = useTranslation()
  const searchRef = useRef<HTMLInputElement>(null)
  const removeRef = useRef<HTMLButtonElement>(null)
  // Set by a change the user made here, so the first render never steals focus.
  const moveFocus = useRef(false)
  const chosen = value !== null

  useEffect(() => {
    if (!moveFocus.current) return
    moveFocus.current = false
    const target = chosen ? removeRef.current : searchRef.current
    target?.focus()
  }, [chosen])

  if (value !== null) {
    return (
      <div className="flex items-center justify-between gap-2 text-sm">
        <span>{value.username}</span>
        <Button
          ref={removeRef}
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => {
            moveFocus.current = true
            onChange(null)
          }}
        >
          {t('devices.manage.ownerClear')}
        </Button>
      </div>
    )
  }
  return (
    <UserCombobox
      inputRef={searchRef}
      label={t('devices.manage.ownerSearch')}
      listLabel={t('devices.manage.ownerCandidates')}
      placeholder={t('devices.manage.ownerNone')}
      disabled={disabled}
      foundText={(count) => t('devices.manage.ownerFound', { count })}
      noMatchText={t('devices.manage.ownerNoMatch')}
      onPick={(user) => {
        moveFocus.current = true
        onChange({ id: user.id, username: user.username })
        return undefined
      }}
    />
  )
}
