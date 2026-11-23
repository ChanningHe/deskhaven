import type { TFunction } from 'i18next'
import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'

import type { Session } from '../session/session'
import type { Quality } from '../settings/settings'
import type { Transport } from './remote-bar'
import { useStore } from './use-remote'

const NONE = '–'

type StatName =
  | 'transport'
  | 'fps'
  | 'kbps'
  | 'delayMs'
  | 'decodeMs'
  | 'decodeQueue'
  | 'keyRequests'
  | 'pointerMoves'
  | 'size'
  | 'codec'
  | 'quality'

function ms(value: number | null): string {
  return value === null ? NONE : `${String(value)} ms`
}

function qualityValue(quality: Quality | null): string | number | null {
  return quality === null || typeof quality === 'string' ? quality : quality.custom
}

function qualityText(quality: Quality | null, t: TFunction): string {
  if (quality === null) return NONE
  if (typeof quality === 'string') return t(`remote.stats.qualities.${quality}`)
  return t('remote.stats.qualities.custom', { level: quality.custom })
}

export function StatsOverlay({
  session,
  transport,
}: {
  session: Session
  transport: Transport | undefined
}) {
  const { t } = useTranslation()
  const s = useStore(session.stats)
  const rows: [stat: StatName, value: string | number | null, text: string][] = [
    ['transport', transport ?? null, transport ? t(`remote.transport.${transport}`) : NONE],
    ['fps', s.fps, String(s.fps)],
    ['kbps', s.kbps, `${String(s.kbps)} kbps`],
    ['delayMs', s.delayMs, ms(s.delayMs)],
    ['decodeMs', s.decodeMs, ms(s.decodeMs)],
    ['decodeQueue', s.decodeQueue, String(s.decodeQueue)],
    ['keyRequests', s.keyRequests, String(s.keyRequests)],
    ['pointerMoves', s.pointerMoves, `${String(s.pointerMoves)}/s`],
    ['size', `${String(s.width)}x${String(s.height)}`, `${String(s.width)}×${String(s.height)}`],
    ['codec', s.codec, s.codec ?? NONE],
    ['quality', qualityValue(s.quality), qualityText(s.quality, t)],
  ]
  return (
    <dl
      aria-label={t('remote.stats.label')}
      className="pointer-events-none absolute top-3 left-3 grid grid-cols-[auto_auto] gap-x-4 rounded-lg bg-black/75 px-3 py-2 font-mono text-[11px] leading-[18px] text-white tabular-nums shadow-lg ring-1 ring-white/10 backdrop-blur-sm"
    >
      {rows.map(([stat, value, text]) => (
        <Fragment key={stat}>
          <dt className="text-white/65">{t(`remote.stats.${stat}`)}</dt>
          <dd data-stat={stat} data-value={value ?? ''} className="text-right">
            {text}
          </dd>
        </Fragment>
      ))}
    </dl>
  )
}
