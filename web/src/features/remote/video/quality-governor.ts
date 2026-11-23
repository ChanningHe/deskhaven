import type { Preset, Quality } from '../settings/settings'

export const START_LEVEL = 12
export const START_QUALITY: Quality = { custom: START_LEVEL }
export const HEALTHY_MS = 100
export const CONGESTED_MS = 200
export const HOLD_TICKS = 3

const GROWTH = 1.5
const BACKOFF = 0.5
const RECOVERY = 0.85
const COOL_TICKS = 3
// rustdesk 1.5.0 src/server/connection.rs:1180-1187 sends the next TestDelay only once the last
// came back, so one missing this long sits in a queue.
const OVERDUE_TICKS = 3

// A preset as a custom level q (bitrate ratio q·2/100): rustdesk 1.5.0
// libs/scrap/src/common/codec.rs:893-895.
const PRESET_LEVEL: Readonly<Record<Preset, number>> = { low: 25, balanced: 33, best: 75 }

export function sameQuality(a: Quality, b: Quality): boolean {
  if (typeof a === 'string' || typeof b === 'string') return a === b
  return a.custom === b.custom
}

function levelOf(quality: Quality): number {
  return typeof quality === 'string' ? PRESET_LEVEL[quality] : quality.custom
}

/**
 * The image quality the peer is asked for during a session, as a ceiling under the user's
 * choice: it starts low, grows while TestDelay stays near the lowest delay seen, and halves
 * when the delay climbs or the next one is overdue. Growth is geometric up to a recovery
 * threshold, then one level per HOLD_TICKS calm delays. The peer's own ABR works below it.
 */
export class QualityGovernor {
  #cap: Quality = START_QUALITY
  #level = START_LEVEL
  #sent: Quality = START_QUALITY
  #threshold = Infinity
  #base: number | null = null
  #previous: number | null = null
  #calm = 0
  #hold = 1
  #sinceCut = Infinity
  #overdue = 0

  get current(): Quality {
    return this.#sent
  }

  setCap(cap: Quality): Quality | null {
    this.#cap = cap
    this.#level = Math.min(this.#level, levelOf(cap))
    return this.#emit()
  }

  tick(delayMs: number | null): Quality | null {
    this.#sinceCut++
    if (delayMs === null) {
      if (this.#base === null || ++this.#overdue < OVERDUE_TICKS) return null
      return this.#congested(true)
    }
    this.#overdue = 0
    const previous = this.#previous ?? delayMs
    this.#previous = delayMs
    this.#base = Math.min(this.#base ?? delayMs, delayMs)
    const excess = delayMs - this.#base
    // The delay lags the queue: a drain under way shows as a falling delay, not a lower one.
    if (excess > CONGESTED_MS) return this.#congested(delayMs >= previous)
    this.#calm = excess > HEALTHY_MS ? 0 : this.#calm + 1
    return this.#calm >= this.#hold ? this.#grow() : null
  }

  #congested(rising: boolean): Quality | null {
    this.#calm = 0
    return this.#sinceCut > COOL_TICKS && rising ? this.#backOff() : null
  }

  #grow(): Quality | null {
    const level = this.#level
    if (level < this.#threshold) {
      this.#level = Math.min(Math.ceil(level * GROWTH), this.#threshold, levelOf(this.#cap))
    } else if (this.#calm % HOLD_TICKS === 0) {
      this.#level = Math.min(level + 1, levelOf(this.#cap))
    }
    return this.#emit()
  }

  #backOff(): Quality | null {
    const level = this.#level
    const floor = Math.min(START_LEVEL, levelOf(this.#cap))
    this.#threshold = Math.floor(level * RECOVERY)
    this.#level = Math.max(floor, Math.floor(level * BACKOFF))
    this.#hold = HOLD_TICKS
    this.#sinceCut = 0
    return this.#emit()
  }

  #emit(): Quality | null {
    const top = this.#level >= levelOf(this.#cap)
    const wanted = top ? this.#cap : { custom: this.#level }
    // Below the cap only the level counts: { custom: 33 } is what 'balanced' already asks for.
    if (top ? sameQuality(wanted, this.#sent) : this.#level === levelOf(this.#sent)) return null
    this.#sent = wanted
    return wanted
  }
}
