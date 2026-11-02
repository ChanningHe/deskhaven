import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'

import { Badge } from '@/components/ui/badge'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

/**
 * Where a value comes from (05 §3: Default / Database / File / Env; 04
 * `managed_by = config` resources are `file`). Map the API's DTO onto this.
 */
export type ValueSource =
  | { kind: 'default' }
  | { kind: 'modified'; by?: string; at?: string }
  | { kind: 'file'; path?: string }
  | { kind: 'env'; name: string }

const BADGE_CLASS = {
  default: 'bg-muted text-muted-foreground',
  modified: 'bg-info/10 text-info',
  locked: 'bg-warning/10 text-warning',
} as const

function labelOf(source: ValueSource, t: TFunction): string {
  if (source.kind === 'default') return t('source.default')
  if (source.kind === 'modified') return t('source.modified')
  return t('source.locked')
}

function formatTime(iso: string, language: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(date)
}

/** The sentence behind the badge: who changed it when, or what locks it. */
function sourceDetail(source: ValueSource, t: TFunction, language: string): string {
  switch (source.kind) {
    case 'default':
      return t('source.defaultDetail')
    case 'modified':
      return source.by !== undefined && source.at !== undefined
        ? t('source.modifiedDetail', { by: source.by, at: formatTime(source.at, language) })
        : t('source.modifiedDetailAnonymous')
    case 'file':
      return source.path === undefined
        ? t('source.fileDetailAnonymous')
        : t('source.fileDetail', { path: source.path })
    case 'env':
      return t('source.envDetail', { name: source.name })
  }
}

/**
 * Source badge (08 §4): grey "default", blue "modified", amber "locked by
 * config". The detail is a tooltip on a focusable trigger, so keyboard and
 * screen reader users get it too.
 */
export function SourceBadge({ source, className }: { source: ValueSource; className?: string }) {
  const { t, i18n } = useTranslation()
  const tone = source.kind === 'file' || source.kind === 'env' ? 'locked' : source.kind
  const label = labelOf(source, t)
  const detail = sourceDetail(source, t, i18n.language)

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Badge
            render={<button type="button" />}
            aria-label={`${label}: ${detail}`}
            className={cn('cursor-default', BADGE_CLASS[tone], className)}
          />
        }
      >
        {label}
      </TooltipTrigger>
      <TooltipContent>{detail}</TooltipContent>
    </Tooltip>
  )
}
