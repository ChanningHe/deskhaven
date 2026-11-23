import { useId, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from '@/components/ui/dropdown-menu'

import {
  MAX_CUSTOM_QUALITY,
  MIN_CUSTOM_QUALITY,
  type Preset,
  type Quality,
} from '../../settings/settings'
import { levelOf } from '../../video/quality-governor'

type Choice = Preset | 'custom'

const CHOICES: readonly Choice[] = ['low', 'balanced', 'best', 'custom']

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

/**
 * The custom level, set when the slider is let go: each change reaches the peer, so a drag
 * must not send one per step.
 */
function CustomLevel({ level, onCommit }: { level: number; onCommit: (level: number) => void }) {
  const { t } = useTranslation()
  const id = useId()
  const [draft, setDraft] = useState<number | null>(null)
  const commit = () => {
    if (draft !== null && draft !== level) onCommit(draft)
    setDraft(null)
  }
  return (
    <div className="flex items-center gap-2 py-1 pr-2 pl-1.5">
      <input
        id={id}
        type="range"
        min={MIN_CUSTOM_QUALITY}
        max={MAX_CUSTOM_QUALITY}
        value={draft ?? level}
        aria-label={t('remote.toolbar.customQuality')}
        className="min-w-0 flex-1 accent-primary"
        onChange={(event) => {
          setDraft(Number(event.target.value))
        }}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        onKeyDown={(event: KeyboardEvent) => {
          if (SLIDER_KEYS.has(event.key)) event.stopPropagation()
        }}
      />
      <output htmlFor={id} className="w-7 text-right text-xs text-muted-foreground tabular-nums">
        {draft ?? level}
      </output>
    </div>
  )
}

export function QualityGroup({
  quality,
  onChange,
}: {
  quality: Quality
  onChange: (quality: Quality) => void
}) {
  const { t } = useTranslation()
  const choice: Choice = typeof quality === 'string' ? quality : 'custom'
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
        {CHOICES.map((option) => (
          <DropdownMenuRadioItem key={option} value={option}>
            {t(`remote.toolbar.qualities.${option}`)}
          </DropdownMenuRadioItem>
        ))}
      </DropdownMenuRadioGroup>
      {typeof quality === 'string' ? null : (
        <CustomLevel
          level={quality.custom}
          onCommit={(level) => {
            onChange({ custom: level })
          }}
        />
      )}
    </DropdownMenuGroup>
  )
}
