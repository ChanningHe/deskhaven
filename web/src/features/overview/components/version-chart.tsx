import type { CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

import type { ClientVersions } from '@/api/generated/model'
import { cn } from '@/lib/utils'

import { versionBars } from '../bars'

/**
 * The client versions as horizontal bars: one series, so one colour; the counts are printed
 * next to the bars. Plain markup (no chart library, no inline scripts: CSP-safe). The
 * versions are what the devices say about themselves, so they are isolated text.
 */
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
    <section aria-labelledby="overview-versions" className="space-y-3">
      <div className="space-y-1">
        <h2 id="overview-versions" className="text-sm font-medium">
          {t('overview.versions.title')}
        </h2>
        <p className="text-xs text-muted-foreground">{t('overview.versions.selfReported')}</p>
      </div>
      {bars.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('overview.versions.empty')}</p>
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
    </section>
  )
}
