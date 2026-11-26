import type { TFunction } from 'i18next'
import { Download, X } from 'lucide-react'
import { Fragment, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

import type { Session } from '../session/session'
import type { Quality } from '../settings/settings'
import { downloadSample, SAMPLE_FRAMES } from '../video/sample-export'
import type { Transport } from './remote-bar'
import { useStore } from './use-remote'

const NONE = '–'

type StatName =
  | 'transport'
  | 'fps'
  | 'sentFps'
  | 'decodeFps'
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

function plain(value: number | null): string {
  return value === null ? NONE : String(value)
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
    ['sentFps', s.sentFps, plain(s.sentFps)],
    ['decodeFps', s.decodeFps, plain(s.decodeFps)],
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
    <div
      role="group"
      aria-label={t('remote.stats.label')}
      className="pointer-events-none absolute top-3 left-3 flex flex-col items-start gap-2"
    >
      <dl className="grid grid-cols-[auto_auto] gap-x-4 rounded-lg bg-black/75 px-3 py-2 font-mono text-[11px] leading-[18px] text-white tabular-nums shadow-lg ring-1 ring-white/10 backdrop-blur-sm">
        {rows.map(([stat, value, text]) => (
          <Fragment key={stat}>
            <dt className="text-white/65">{t(`remote.stats.${stat}`)}</dt>
            <dd data-stat={stat} data-value={value ?? ''} className="text-right">
              {text}
            </dd>
          </Fragment>
        ))}
      </dl>
      <SampleButton session={session} />
    </div>
  )
}

/** Records raw frames for a test fixture and saves them as a local file; nothing is uploaded. */
function SampleButton({ session }: { session: Session }) {
  const { t } = useTranslation()
  const sampling = useStore(session.sampling)
  const frames = sampling.kind === 'recording' ? sampling.frames : null
  const total = SAMPLE_FRAMES
  return (
    <button
      type="button"
      title={t('remote.stats.sampleHint', { total })}
      onClick={() => {
        if (frames !== null) {
          session.cancelSample()
          return
        }
        void session.recordSample().then((sample) => {
          if (sample) downloadSample(sample)
        })
      }}
      className="pointer-events-auto relative flex items-center gap-1.5 overflow-hidden rounded-lg bg-black/75 px-2.5 py-1 text-[11px] leading-[18px] font-medium text-white tabular-nums shadow-lg ring-1 ring-white/10 backdrop-blur-sm transition-colors hover:bg-black/85 focus-visible:ring-2 focus-visible:ring-white/70 focus-visible:outline-none"
    >
      {frames === null ? (
        <>
          <Download aria-hidden className="size-3.5" />
          {t('remote.stats.sampleExport')}
        </>
      ) : (
        <>
          <span
            aria-hidden
            className="absolute inset-y-0 left-0 w-(--sampled) bg-white/15 transition-[width]"
            style={{ '--sampled': `${String((frames / total) * 100)}%` } as CSSProperties}
          />
          <X aria-hidden className="relative size-3.5" />
          <span className="relative">
            {frames === 0
              ? t('remote.stats.sampleWaiting')
              : t('remote.stats.sampleRecording', { frames, total })}
            <span className="text-white/65"> · {t('remote.stats.sampleCancel')}</span>
          </span>
        </>
      )}
    </button>
  )
}
