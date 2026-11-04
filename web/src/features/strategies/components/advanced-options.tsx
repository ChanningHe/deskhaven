import { X } from 'lucide-react'
import { useId, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { KEY_MAX_LENGTH, keyProblem, setTo, VALUE_MAX_LENGTH, valueProblem } from '../option-draft'
import type { KeyProblem, OptionValue, ValueProblem } from '../option-draft'

interface AdvancedOptionsProps {
  /** Options outside the typed form that the draft sets, by name. */
  entries: [string, OptionValue][]
  /** Option names the typed form above covers. */
  known: Set<string>
  /** Option names whose value is never shown (password inputs, withheld saved values). */
  secret: Set<string>
  problems: Map<string, ValueProblem>
  onChange: (key: string, value: OptionValue | undefined) => void
  disabled: boolean
}

/** Free option names and values, for options the typed form does not cover. */
export function AdvancedOptions(props: AdvancedOptionsProps) {
  const { entries, problems } = props
  const { t } = useTranslation()
  return (
    <fieldset className="grid gap-2 rounded-lg border p-3">
      <legend className="px-1 text-sm font-medium">{t('strategies.advanced.title')}</legend>
      <p className="text-xs text-muted-foreground">{t('strategies.advanced.description')}</p>
      {entries.map(([key, value]) => (
        <div key={key} className="grid gap-1">
          <FreeRow {...props} optionKey={key} value={value} />
          <ProblemLine problem={problems.get(key)} />
        </div>
      ))}
      {props.disabled ? null : <AddOption {...props} />}
    </fieldset>
  )
}

interface FreeRowProps extends AdvancedOptionsProps {
  optionKey: string
  value: OptionValue
}

/** One free option: its value (a password input for secrets, a note for a withheld one). */
function FreeRow({ optionKey: key, value, secret, onChange, disabled }: FreeRowProps) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2">
      <code className="min-w-0 flex-1 truncate text-xs">{key}</code>
      {value.kind === 'kept' ? (
        <span className="flex flex-[2] items-center gap-2 text-xs text-muted-foreground">
          {t('strategies.option.keptHint')}
          {disabled ? null : (
            <Button
              type="button"
              variant="outline"
              size="xs"
              onClick={() => {
                onChange(key, setTo(''))
              }}
            >
              {t('strategies.advanced.replace')}
            </Button>
          )}
        </span>
      ) : (
        <Input
          aria-label={t('strategies.option.valueOf', { name: key })}
          className="flex-[2]"
          type={secret.has(key) ? 'password' : 'text'}
          autoComplete="off"
          maxLength={VALUE_MAX_LENGTH}
          disabled={disabled}
          value={value.kind === 'set' ? value.value : ''}
          onChange={(e) => {
            onChange(key, setTo(e.target.value))
          }}
        />
      )}
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={t('strategies.advanced.remove', { key })}
        disabled={disabled}
        onClick={() => {
          onChange(key, undefined)
        }}
      >
        <X aria-hidden />
      </Button>
    </div>
  )
}

function ProblemLine({ problem }: { problem: ValueProblem | undefined }) {
  const { t } = useTranslation()
  if (problem === undefined) return null
  return (
    <p role="alert" className="text-xs text-destructive">
      {t(`strategies.problems.${problem}`)}
    </p>
  )
}

type AddProblem =
  | `strategies.advanced.problems.${KeyProblem | 'known' | 'taken'}`
  | `strategies.problems.${ValueProblem}`

/** Why `key` cannot be added, as an i18n key. */
function addProblem(
  key: string,
  value: string,
  taken: (key: string) => boolean,
  known: Set<string>,
): AddProblem | undefined {
  const problem = keyProblem(key)
  if (problem !== undefined) return `strategies.advanced.problems.${problem}`
  if (known.has(key)) return 'strategies.advanced.problems.known'
  if (taken(key)) return 'strategies.advanced.problems.taken'
  const invalid = valueProblem(undefined, value)
  return invalid === undefined ? undefined : `strategies.problems.${invalid}`
}

function AddOption({ entries, known, secret, onChange }: AdvancedOptionsProps) {
  const { t } = useTranslation()
  const id = useId()
  const [key, setKey] = useState('')
  const [value, setValue] = useState('')
  const [error, setError] = useState<AddProblem | undefined>(undefined)

  const add = () => {
    const trimmed = key.trim()
    const problem = addProblem(trimmed, value, (k) => entries.some(([e]) => e === k), known)
    setError(problem)
    if (problem !== undefined) return
    onChange(trimmed, setTo(value))
    setKey('')
    setValue('')
  }
  // Enter adds the option; it must not submit the editor form around it.
  const onEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    add()
  }

  return (
    <div className="grid gap-1 border-t pt-2">
      <div className="flex flex-wrap items-end gap-2">
        <div className="grid flex-1 gap-1">
          <Label htmlFor={`${id}-key`}>{t('strategies.advanced.key')}</Label>
          <Input
            id={`${id}-key`}
            maxLength={KEY_MAX_LENGTH}
            value={key}
            onKeyDown={onEnter}
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="grid flex-[2] gap-1">
          <Label htmlFor={`${id}-value`}>{t('strategies.advanced.value')}</Label>
          <Input
            id={`${id}-value`}
            type={secret.has(key.trim()) ? 'password' : 'text'}
            autoComplete="off"
            maxLength={VALUE_MAX_LENGTH}
            value={value}
            onKeyDown={onEnter}
            onChange={(e) => {
              setValue(e.target.value)
            }}
          />
        </div>
        <Button type="button" variant="outline" size="sm" onClick={add}>
          {t('strategies.advanced.add')}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t('strategies.advanced.hint')}</p>
      {error === undefined ? null : (
        <p role="alert" className="text-xs text-destructive">
          {t(error)}
        </p>
      )}
    </div>
  )
}
