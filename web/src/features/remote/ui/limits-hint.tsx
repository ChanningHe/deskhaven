import { CircleQuestionMark } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { IconButton } from './toolbar-controls'
import { type BrowserAbilities, limitHints } from './limit-hints'

/** What this browser cannot pass on to the device, shown on hover or focus. */
export function LimitsHint(abilities: BrowserAbilities) {
  const { t } = useTranslation()
  const label = t('remote.limits.label')
  return (
    <IconButton
      label={label}
      hint={
        <ul aria-label={label} className="list-disc space-y-1 py-0.5 pl-4 leading-relaxed">
          {limitHints(abilities).map((line) => (
            <li key={line}>{t(line)}</li>
          ))}
        </ul>
      }
    >
      <CircleQuestionMark aria-hidden />
    </IconButton>
  )
}
