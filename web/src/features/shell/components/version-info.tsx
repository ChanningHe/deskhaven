import { CircleArrowUp } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import type { AvailableUpdate } from '../update'
import { useServerVersion } from '../version'

interface VersionInfoProps {
  className?: string
  update?: AvailableUpdate | undefined
}

/** The source link is required by AGPL-3.0 section 13. */
export function VersionInfo({ className, update }: VersionInfoProps) {
  const { t } = useTranslation()
  const info = useServerVersion()
  if (info === undefined) return null

  return (
    <p className={cn('flex text-xs text-muted-foreground', className)}>
      <a
        href={info.sourceUrl}
        target="_blank"
        rel="noopener noreferrer"
        title={t('shell.sourceCode')}
        className="truncate underline-offset-4 hover:text-foreground hover:underline"
      >
        v{info.version}
      </a>
      {update === undefined ? null : <UpdateLink update={update} />}
    </p>
  )
}

function UpdateLink({ update }: { update: AvailableUpdate }) {
  const { t } = useTranslation()
  const label = t('shell.updateAvailable', { version: update.version })

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <a
            href={update.releaseUrl}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={label}
            className="ml-1.5 shrink-0 self-center rounded-sm text-info outline-none hover:opacity-80 focus-visible:ring-2 focus-visible:ring-ring"
          />
        }
      >
        <CircleArrowUp aria-hidden className="size-3.5" />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
