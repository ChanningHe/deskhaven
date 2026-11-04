import type { CSSProperties, ReactNode } from 'react'

import { cn } from '@/lib/utils'

interface StatTileProps {
  label: string
  /** The headline number, already formatted. */
  value: ReactNode
  /** Small print under the number (the window, what the number counts). */
  hint?: ReactNode
  /** A share to draw under the number: `value` of `max`. */
  meter?: { value: number; max: number; label: string }
  /** Draw the number in the warning colour (something needs a look). */
  attention?: boolean
}

/** One headline number of the overview. */
export function StatTile({ label, value, hint, meter, attention = false }: StatTileProps) {
  return (
    <div className="space-y-2 rounded-lg border bg-card p-4 text-card-foreground">
      <p className="text-sm text-muted-foreground">{label}</p>
      <p className={cn('text-3xl font-semibold tabular-nums', attention && 'text-warning')}>
        {value}
      </p>
      {meter === undefined ? null : <Meter {...meter} />}
      {hint === undefined ? null : <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

function Meter({ value, max, label }: { value: number; max: number; label: string }) {
  const share = max <= 0 ? 0 : Math.min(100, Math.round((value / max) * 100))
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={value}
      className="h-2 overflow-hidden rounded-full bg-muted"
    >
      <div
        className="h-full w-(--share) rounded-full bg-primary"
        style={{ '--share': `${String(share)}%` } as CSSProperties}
      />
    </div>
  )
}
