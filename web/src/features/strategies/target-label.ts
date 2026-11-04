import type { TFunction } from 'i18next'

import type { AssignmentView } from '@/api/generated/model'

/** "Device group: Office", "Global", or the id of a target whose row is gone. */
export function targetLabel(a: AssignmentView, t: TFunction): string {
  const kind = t(`strategies.targets.${a.target_type}`)
  if (a.target_type === 'global') return kind
  const name = a.target_name ?? t('strategies.assignments.missing', { id: a.target_id })
  return t('strategies.assignments.target', { kind, name })
}
