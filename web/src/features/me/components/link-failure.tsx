import { useTranslation } from 'react-i18next'

import { ErrorAlert } from '@/components/common/error-alert'

import { linkReasonOf } from '../link-reasons'
import type { LinkReason } from '../link-reasons'

/** A refused link as its specific reason; any other error as the generic alert. */
export function LinkFailure({ error, reason }: { error?: unknown; reason?: LinkReason }) {
  const { t } = useTranslation()
  const known = reason ?? linkReasonOf(error)
  if (known === undefined) return <ErrorAlert error={error} />
  return (
    <p role="alert" className="text-sm text-destructive">
      {t(`me.identities.reasons.${known}`)}
    </p>
  )
}
