import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

interface StrategyDetailsProps {
  draft: { name: string; note: string }
  onChange: (change: Partial<{ name: string; note: string }>) => void
  disabled: boolean
}

/** Name and note of a strategy. */
export function StrategyDetails({ draft, onChange, disabled }: StrategyDetailsProps) {
  const { t } = useTranslation()
  const id = useId()
  return (
    <div className="grid gap-2 sm:grid-cols-2">
      <div className="grid gap-1">
        <Label htmlFor={`${id}-name`}>{t('strategies.fields.name')}</Label>
        <Input
          id={`${id}-name`}
          required
          maxLength={64}
          disabled={disabled}
          value={draft.name}
          onChange={(e) => {
            onChange({ name: e.target.value })
          }}
        />
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`${id}-note`}>{t('strategies.fields.note')}</Label>
        <Input
          id={`${id}-note`}
          maxLength={2000}
          disabled={disabled}
          value={draft.note}
          onChange={(e) => {
            onChange({ note: e.target.value })
          }}
        />
      </div>
    </div>
  )
}
