import { useId, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { Trans, useTranslation } from 'react-i18next'

import {
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from '@/components/ui/dropdown-menu'

import type { Size } from '../../session/resolution'
import {
  MAX_CUSTOM_QUALITY,
  MIN_CUSTOM_QUALITY,
  type Preset,
  type Quality,
} from '../../settings/settings'
import { bitrateModeled, formatKbps, formatRatio, ratioOf, targetKbps } from '../../video/bitrate'
import { levelOf } from '../../video/quality-governor'

type Choice = Preset | 'custom'
type Rate = (quality: Quality) => string | null

const CHOICES: readonly Choice[] = ['low', 'balanced', 'best', 'custom']

const FINE_UP_TO = 100
const FINE_STEP = 5
const COARSE_STEP = 25

function levels(from: number, to: number, step: number): number[] {
  return Array.from({ length: Math.floor((to - from) / step) + 1 }, (_, i) => from + i * step)
}

/** Slider positions: fine steps where the presets sit, coarse ones above. */
const CUSTOM_STEPS: readonly number[] = [
  ...levels(MIN_CUSTOM_QUALITY, FINE_UP_TO, FINE_STEP),
  ...levels(FINE_UP_TO + COARSE_STEP, MAX_CUSTOM_QUALITY, COARSE_STEP),
]

function positionOf(level: number): number {
  const off = (i: number) => Math.abs((CUSTOM_STEPS[i] ?? level) - level)
  return CUSTOM_STEPS.reduce((nearest, _, i) => (off(i) < off(nearest) ? i : nearest), 0)
}

const SLIDER_KEYS = new Set([
  'ArrowLeft',
  'ArrowRight',
  'ArrowUp',
  'ArrowDown',
  'Home',
  'End',
  'PageUp',
  'PageDown',
])

/** Set when the slider is let go: each change reaches the peer, so a drag must not send one per step. */
function CustomLevel({
  level,
  rate,
  onCommit,
}: {
  level: number
  rate: Rate
  onCommit: (level: number) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const [draft, setDraft] = useState<number | null>(null)
  const shown = draft ?? level
  const shownRate = rate({ custom: shown })
  const commit = () => {
    if (draft !== null && draft !== level) onCommit(draft)
    setDraft(null)
  }
  return (
    <div className="flex items-center gap-2 py-1 pr-2 pl-1.5">
      <input
        id={id}
        type="range"
        min={0}
        max={CUSTOM_STEPS.length - 1}
        step={1}
        value={positionOf(shown)}
        aria-label={t('remote.toolbar.customQuality')}
        aria-valuetext={
          shownRate === null
            ? String(shown)
            : t('remote.toolbar.customQualityValue', { level: shown, rate: shownRate })
        }
        className="min-w-0 flex-1 accent-primary"
        onChange={(event) => {
          setDraft(CUSTOM_STEPS[Number(event.target.value)] ?? level)
        }}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        onKeyDown={(event: KeyboardEvent) => {
          if (SLIDER_KEYS.has(event.key)) event.stopPropagation()
        }}
      />
      <output htmlFor={id} className="w-7 text-right text-xs tabular-nums">
        {shown}
      </output>
      {shownRate === null ? null : (
        <span className="w-[4.5rem] text-right text-xs text-muted-foreground tabular-nums">
          {shownRate}
        </span>
      )}
    </div>
  )
}

export function QualityGroup({
  quality,
  size,
  codec,
  onChange,
}: {
  quality: Quality
  size: Size | null
  codec: string | null
  onChange: (quality: Quality) => void
}) {
  const { t } = useTranslation()
  const choice: Choice = typeof quality === 'string' ? quality : 'custom'
  const modeled = bitrateModeled(codec)
  const rate: Rate = (q) => {
    if (!modeled) return null
    return size ? formatKbps(targetKbps(q, size.width, size.height)) : formatRatio(ratioOf(q))
  }
  return (
    <DropdownMenuGroup>
      <DropdownMenuLabel>{t('remote.toolbar.quality')}</DropdownMenuLabel>
      <DropdownMenuRadioGroup
        value={choice}
        onValueChange={(value: unknown) => {
          const picked = CHOICES.find((c) => c === value)
          if (picked === undefined || picked === choice) return
          onChange(picked === 'custom' ? { custom: levelOf(quality) } : picked)
        }}
      >
        {CHOICES.map((option) => {
          const name = t(`remote.toolbar.qualities.${option}`)
          const optionRate = option === 'custom' ? null : rate(option)
          return (
            <DropdownMenuRadioItem key={option} value={option}>
              {optionRate === null ? (
                name
              ) : (
                <span>
                  <Trans
                    i18nKey="remote.toolbar.qualityOption"
                    values={{ name, rate: optionRate }}
                    components={{ rate: <span className="text-muted-foreground" /> }}
                  />
                </span>
              )}
            </DropdownMenuRadioItem>
          )
        })}
      </DropdownMenuRadioGroup>
      {typeof quality === 'string' ? null : (
        <CustomLevel
          level={quality.custom}
          rate={rate}
          onCommit={(level) => {
            onChange({ custom: level })
          }}
        />
      )}
    </DropdownMenuGroup>
  )
}
