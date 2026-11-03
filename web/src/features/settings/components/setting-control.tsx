import { useTranslation } from 'react-i18next'

import type { SettingView } from '@/api/generated/model'
import { NativeSelect } from '@/components/common/native-select'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import { isSecretSet } from '../editor'
import type { Editor } from '../editor'

interface SettingControlProps {
  setting: SettingView
  /** Element id, for the row's label and help text. */
  id: string
  editor: Editor
  disabled: boolean
  invalid: boolean
  describedBy: string
  onChange: (editor: Editor) => void
}

const TEXTAREA_CLASS =
  'min-h-16 w-full rounded-lg border border-input bg-transparent px-2.5 py-1.5 font-mono text-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive dark:bg-input/30'

/** The form control for one setting, chosen by its schema type (read-only when `disabled`). */
export function SettingControl({
  setting,
  id,
  editor,
  disabled,
  invalid,
  describedBy,
  onChange,
}: SettingControlProps) {
  const { t } = useTranslation()
  const { schema } = setting
  const shared = {
    id,
    disabled,
    'aria-invalid': invalid || undefined,
    'aria-describedby': describedBy,
  }
  switch (schema.type) {
    case 'bool':
      return (
        <label className="flex items-center gap-2 text-sm">
          <Checkbox
            {...shared}
            checked={editor === true}
            onCheckedChange={(checked) => {
              onChange(checked)
            }}
          />
          {t('settings.control.enabled')}
        </label>
      )
    case 'enum':
      return (
        <NativeSelect
          {...shared}
          className="w-full sm:w-56"
          value={String(editor)}
          onChange={(event) => {
            onChange(event.target.value)
          }}
        >
          {(schema.variants ?? []).map((variant) => (
            <option key={variant} value={variant}>
              {variant}
            </option>
          ))}
        </NativeSelect>
      )
    case 'host_port_list':
      return (
        <textarea
          {...shared}
          className={TEXTAREA_CLASS}
          rows={3}
          spellCheck={false}
          placeholder={t('settings.control.listPlaceholder')}
          value={String(editor)}
          onChange={(event) => {
            onChange(event.target.value)
          }}
        />
      )
    case 'secret':
      return (
        <Input
          {...shared}
          type="password"
          autoComplete="new-password"
          className="sm:max-w-sm"
          placeholder={
            isSecretSet(setting)
              ? t('settings.control.secretKeep')
              : t('settings.control.secretEmpty')
          }
          value={String(editor)}
          onChange={(event) => {
            onChange(event.target.value)
          }}
        />
      )
    default:
      return (
        <Input
          {...shared}
          type="text"
          inputMode={schema.type === 'u32' ? 'numeric' : undefined}
          autoComplete="off"
          spellCheck={false}
          className={cn(schema.type === 'string' ? 'sm:max-w-sm' : 'sm:w-48')}
          value={String(editor)}
          onChange={(event) => {
            onChange(event.target.value)
          }}
        />
      )
  }
}
