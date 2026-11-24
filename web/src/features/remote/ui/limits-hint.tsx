import { Popover } from '@base-ui/react/popover'
import { CircleQuestionMark } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { type BrowserAbilities, limitHints } from './limit-hints'
import { ToolbarPopover } from './toolbar-controls'

/** What this browser cannot pass on to the device. */
export function LimitsHint(abilities: BrowserAbilities) {
  const { t } = useTranslation()
  return (
    <ToolbarPopover label={t('remote.limits.label')} icon={<CircleQuestionMark aria-hidden />}>
      <Popover.Description
        render={<ul className="list-disc space-y-1 pl-4 leading-relaxed text-muted-foreground" />}
      >
        {limitHints(abilities).map((line) => (
          <li key={line}>{t(line)}</li>
        ))}
      </Popover.Description>
    </ToolbarPopover>
  )
}
