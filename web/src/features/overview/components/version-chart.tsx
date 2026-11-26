import { ChartNoAxesColumn } from 'lucide-react'
import type { CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

import type { ClientVersions } from '@/api/generated/model'
import { EmptyState } from '@/components/common/empty-state'
import { SectionCard } from '@/components/common/section-card'
import { cn } from '@/lib/utils'

import { versionBars } from '../bars'

/** Plain markup (no chart library, no inline scripts) for the CSP; versions are isolated text. */
export function VersionChart({
  versions,
  format,
}: {
  versions: ClientVersions
  format: (n: number) => string
}) {
  const { t } = useTranslation()
  const bars = versionBars(versions)
  return (
    <SectionCard
      title={t('overview.versions.title')}
      description={t('overview.versions.selfReported')}
    >
      {bars.length === 0 ? (
        <EmptyState
          icon={ChartNoAxesColumn}
          headingLevel={3}
          title={t('overview.versions.empty')}
        />
      ) : (
        <ul className="space-y-2">
          {bars.map((bar) => (
            <li
              key={bar.key}
              className="grid grid-cols-[minmax(4rem,8rem)_1fr_auto] items-center gap-3 text-sm"
            >
              <span className="truncate font-mono text-xs">
                {bar.kind === 'version' ? (
                  <bdi>{bar.version}</bdi>
                ) : (
                  t(`overview.versions.${bar.kind}`)
                )}
              </span>
              <span className="h-3 overflow-hidden rounded-sm bg-muted" aria-hidden>
                <span
                  className={cn(
                    'block h-full w-(--share) rounded-r-sm',
                    bar.kind === 'version' ? 'bg-primary' : 'bg-muted-foreground/50',
                  )}
                  style={{ '--share': `${String(bar.width)}%` } as CSSProperties}
                />
              </span>
              <span className="tabular-nums">{format(bar.count)}</span>
            </li>
          ))}
        </ul>
      )}
    </SectionCard>
  )
}
