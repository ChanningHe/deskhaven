import { useId } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import type { CatalogOptionView } from '@/api/generated/model'
import { NativeSelect } from '@/components/common/native-select'
import { Input } from '@/components/ui/input'

import { textOf } from '../catalog'
import { KEPT, RESET, setTo } from '../option-draft'
import type { OptionValue, ValueProblem } from '../option-draft'

interface OptionFieldProps {
  option: CatalogOptionView
  value: OptionValue | undefined
  onChange: (value: OptionValue | undefined) => void
  /** The server holds a withheld secret for this option. */
  hasSaved: boolean
  problem: ValueProblem | undefined
  disabled: boolean
}

/** Whether the option is edited with a select of its values (bool, choice). */
function isFixed(option: CatalogOptionView): boolean {
  return option.kind === 'bool' || option.kind === 'choice'
}

/** `<select>` value of a draft value: '' unmanaged, `reset`, `kept`, `set` or `set:<v>`. */
function encode(value: OptionValue | undefined, fixed: boolean): string {
  if (value === undefined) return ''
  if (value.kind !== 'set') return value.kind
  return fixed ? `set:${value.value}` : 'set'
}

function decode(raw: string, option: CatalogOptionView): OptionValue | undefined {
  if (raw === '') return undefined
  if (raw === 'reset') return RESET
  if (raw === 'kept') return KEPT
  if (raw.startsWith('set:')) return setTo(raw.slice('set:'.length))
  return setTo(option.kind === 'number' ? (option.default ?? '') : '')
}

/** The label of a value of `option` (Y / N, a choice) or the value itself. */
function valueLabel(option: CatalogOptionView, value: string, t: TFunction, language: string) {
  if (option.kind === 'bool') {
    return value === 'Y' ? t('strategies.option.on') : t('strategies.option.off')
  }
  const choice = option.choices.find((c) => c.value === value)
  return choice === undefined ? value : textOf(choice.label, language)
}

/** The choices of the mode select. */
function ModeOptions({ option, value, hasSaved }: OptionFieldProps) {
  const { t, i18n } = useTranslation()
  const fixed = isFixed(option)
  const values = option.kind === 'bool' ? ['Y', 'N'] : option.choices.map((c) => c.value)
  const current = value?.kind === 'set' ? value.value : undefined
  const unknown = fixed && current !== undefined && !values.includes(current)
  return (
    <>
      {hasSaved ? <option value="kept">{t('strategies.option.kept')}</option> : null}
      <option value="">{t('strategies.option.unmanaged')}</option>
      {fixed ? (
        values.map((v) => (
          <option key={v} value={`set:${v}`}>
            {valueLabel(option, v, t, i18n.language)}
          </option>
        ))
      ) : (
        <option value="set">{t('strategies.option.set')}</option>
      )}
      {unknown ? <option value={`set:${current}`}>{current}</option> : null}
      <option value="reset">{t('strategies.option.reset')}</option>
    </>
  )
}

/** Description and client default of an option. */
function OptionHelp({ option }: { option: CatalogOptionView }) {
  const { t, i18n } = useTranslation()
  const description = textOf(option.description, i18n.language)
  return (
    <>
      {description === '' ? null : <p className="text-xs text-muted-foreground">{description}</p>}
      {option.default == null ? null : (
        <p className="text-xs text-muted-foreground">
          {t('strategies.option.default', {
            value: valueLabel(option, option.default, t, i18n.language),
          })}
        </p>
      )}
    </>
  )
}

/** The value input of a text or number option that is set. */
function ValueInput({ option, value, onChange, disabled }: OptionFieldProps) {
  const { t, i18n } = useTranslation()
  if (isFixed(option) || value?.kind !== 'set') return null
  return (
    <Input
      aria-label={t('strategies.option.valueOf', { name: textOf(option.label, i18n.language) })}
      className="max-w-sm flex-1"
      type={option.secret ? 'password' : 'text'}
      inputMode={option.kind === 'number' ? 'numeric' : undefined}
      autoComplete="off"
      disabled={disabled}
      value={value.value}
      onChange={(e) => {
        onChange(setTo(e.target.value))
      }}
    />
  )
}

/** One catalog option: label, description, client default and a typed control. */
export function OptionField(props: OptionFieldProps) {
  const { option, value, onChange, problem, disabled } = props
  const { t, i18n } = useTranslation()
  const id = useId()
  return (
    <div className="grid gap-1 border-t py-3 first:border-t-0">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <label htmlFor={`${id}-mode`} className="text-sm font-medium">
          {textOf(option.label, i18n.language)}
        </label>
        <code className="text-xs text-muted-foreground">{option.key}</code>
      </div>
      <OptionHelp option={option} />
      <div className="flex flex-wrap items-center gap-2">
        <NativeSelect
          id={`${id}-mode`}
          value={encode(value, isFixed(option))}
          disabled={disabled}
          aria-invalid={problem === undefined ? undefined : true}
          onChange={(e) => {
            onChange(decode(e.target.value, option))
          }}
        >
          <ModeOptions {...props} />
        </NativeSelect>
        <ValueInput {...props} />
        {value?.kind === 'kept' ? (
          <span className="text-xs text-muted-foreground">{t('strategies.option.keptHint')}</span>
        ) : null}
      </div>
      {problem === undefined ? null : (
        <p role="alert" className="text-xs text-destructive">
          {t(`strategies.problems.${problem}`)}
        </p>
      )}
    </div>
  )
}
