import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'

interface SelectAllCheckboxProps {
  checked: boolean
  indeterminate: boolean
  onChange: (checked: boolean) => void
}

/** Header checkbox: selects every row on the current page. */
export function SelectAllCheckbox({ checked, indeterminate, onChange }: SelectAllCheckboxProps) {
  const { t } = useTranslation()
  return (
    <Checkbox
      aria-label={t('dataTable.selectAll')}
      checked={checked}
      indeterminate={indeterminate}
      onCheckedChange={onChange}
    />
  )
}

interface SelectRowCheckboxProps {
  /** The row's name ("Select <name>"); a generic label without it. */
  name: string | undefined
  checked: boolean
  onChange: (checked: boolean) => void
}

export function SelectRowCheckbox({ name, checked, onChange }: SelectRowCheckboxProps) {
  const { t } = useTranslation()
  const label =
    name === undefined ? t('dataTable.selectRow') : t('dataTable.selectRowNamed', { name })
  return <Checkbox aria-label={label} checked={checked} onCheckedChange={onChange} />
}
