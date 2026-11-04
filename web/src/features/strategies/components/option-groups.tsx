import { useTranslation } from 'react-i18next'

import type { CatalogOptionView } from '@/api/generated/model'

import { grouped } from '../catalog'
import type { OptionDraft, OptionValue, ValueProblem } from '../option-draft'
import { OptionField } from './option-field'

interface OptionGroupsProps {
  options: CatalogOptionView[]
  draft: OptionDraft
  /** Secret options whose value the server holds and withholds. */
  saved: Set<string>
  problems: Map<string, ValueProblem>
  onChange: (key: string, value: OptionValue | undefined) => void
  disabled: boolean
}

/** The catalog options, one section per group (security, permissions, …). */
export function OptionGroups({
  options,
  draft,
  saved,
  problems,
  onChange,
  disabled,
}: OptionGroupsProps) {
  const { t } = useTranslation()
  return (
    <>
      {grouped(options).map(({ group, options: inGroup }) => (
        <fieldset key={group} className="rounded-lg border px-3 pb-1">
          <legend className="px-1 text-sm font-medium">{t(`strategies.groups.${group}`)}</legend>
          {inGroup.map((option) => (
            <OptionField
              key={option.key}
              option={option}
              value={draft[option.key]}
              hasSaved={saved.has(option.key)}
              problem={problems.get(option.key)}
              disabled={disabled}
              onChange={(value) => {
                onChange(option.key, value)
              }}
            />
          ))}
        </fieldset>
      ))}
    </>
  )
}
