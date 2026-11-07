import { useTranslation } from 'react-i18next'

import type { SettingView } from '@/api/generated/model'

import type { Editor, Problem } from '../editor'

/** The accepted range or shape of a value, under its control. */
export function RangeHint({ setting }: { setting: SettingView }) {
  const { t } = useTranslation()
  const { schema } = setting
  let hint: string | undefined
  if (schema.type === 'u32' || schema.type === 'duration') {
    const range = { min: String(schema.min), max: String(schema.max) }
    hint =
      schema.zero_allowed === true
        ? t('settings.hint.rangeOrZero', range)
        : t('settings.hint.range', range)
  } else if (schema.type === 'host_port_list') {
    hint = t('settings.hint.list', { max: schema.max_items ?? 0 })
  } else if (schema.type === 'secret') {
    hint = t('settings.hint.secret')
  }
  return hint === undefined ? null : <p className="text-xs text-muted-foreground">{hint}</p>
}

/** Values of a setting that hand an outside party power over privileged accounts. */
const RISKY_VALUES: Readonly<Record<string, readonly string[]>> = {
  'auth.oidc.link_email_max_role': ['admin', 'owner'],
}

/** A standing warning while the shown value is a risky one (see `RISKY_VALUES`). */
export function RiskHint({ setting, editor }: { setting: SettingView; editor: Editor }) {
  const { t } = useTranslation()
  const risky = RISKY_VALUES[setting.key] ?? []
  if (typeof editor !== 'string' || !risky.includes(editor)) return null
  return (
    <p role="note" className="text-xs text-warning">
      {t('settings.risk.linkEmailMaxRole')}
    </p>
  )
}

/** A client-side finding about the edited value, as a message. */
export function ProblemText({ problem }: { problem: Problem }) {
  const { t } = useTranslation()
  let message: string
  switch (problem.kind) {
    case 'required':
      message = t('settings.problem.required')
      break
    case 'whole_number':
      message = t('settings.problem.wholeNumber')
      break
    case 'range':
      message = t('settings.problem.range', { min: problem.min, max: problem.max })
      break
    case 'duration':
      message = t('settings.problem.duration')
      break
    case 'max_length':
      message = t('settings.problem.maxLength', { max: problem.max })
      break
    case 'max_items':
      message = t('settings.problem.maxItems', { max: problem.max })
      break
    case 'variant':
      message = t('settings.problem.variant')
      break
  }
  return (
    <p role="alert" className="text-xs text-destructive">
      {message}
    </p>
  )
}
