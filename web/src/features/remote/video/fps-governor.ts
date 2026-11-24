export const DEFAULT_FPS = 30
export const INACTIVE_TICKS = 15
export const REFRESH_INTERVAL_MS = 10_000
export const MAX_REFRESHES = 20

const WARM_UP_FRAMES = 3
const RATE_EVERY = 10
const RATE_WINDOW = 30
const IDLE_TICKS = 3
const STEP_UP = 3

export interface FpsGovernorOptions {
  direct: boolean
  userFps: number
}

export interface FpsTick {
  fps: number
  queued: number
}

// rustdesk 1.5.1 src/client/io_loop.rs:1557-1655, decode rate src/client.rs:4164-4193.
export class FpsGovernor {
  readonly #tenths: number
  readonly #startFps: number
  #userFps: number
  #userChanged = false
  #warmUp = 0
  #count = 0
  #totalMs = 0
  #decodeFps: number | null = null
  #lastFps: number | null = null
  #idle = 0
  #inactive = 0
  #refreshes = 0
  #lastRefreshAt = 0

  constructor(options: FpsGovernorOptions) {
    this.#tenths = options.direct ? 9 : 8
    this.#startFps = options.userFps
    this.#userFps = options.userFps
  }

  setUserFps(fps: number): void {
    if (fps === this.#userFps) return
    this.#userFps = fps
    this.#userChanged = true
    this.#idle = 0
  }

  decoded(serviceMs: number): void {
    if (this.#warmUp < WARM_UP_FRAMES) {
      this.#warmUp++
      return
    }
    this.#count++
    this.#totalMs += serviceMs
    if (this.#count % RATE_EVERY === 0 && this.#totalMs > 0) {
      this.#decodeFps = Math.floor((this.#count * 1000) / this.#totalMs)
    }
    if (this.#count >= RATE_WINDOW) {
      this.#count = 0
      this.#totalMs = 0
    }
  }

  tick({ fps, queued }: FpsTick): number | null {
    this.#inactive = fps === 0 ? this.#inactive + 1 : 0
    const decodeFps = this.#decodeFps
    if (decodeFps === null || this.#inactive >= INACTIVE_TICKS) return this.#lowered()
    const limited = Math.min(Math.floor((decodeFps * this.#tenths) / 10), this.#userFps)
    const trend = this.#trend(queued, decodeFps, limited)
    const changed = this.#userChanged
    this.#userChanged = false
    if (this.#lastFps !== null && trend === null && !changed) return null
    let next = limited
    if (trend === 'down' && limited < queued) next = Math.floor(limited / 2)
    next = Math.max(1, next)
    if (next === this.#lastFps) return null
    this.#lastFps = next
    return next
  }

  // Without a decode rate the decoder's headroom is unknown, so only a lower ceiling goes out now.
  #lowered(): number | null {
    if (!this.#userChanged || this.#userFps >= (this.#lastFps ?? this.#startFps)) return null
    this.#userChanged = false
    this.#lastFps = this.#userFps
    return this.#userFps
  }

  mayRefresh(now: number): boolean {
    if (this.#refreshes >= MAX_REFRESHES) return false
    if (this.#refreshes > 0 && now - this.#lastRefreshAt <= REFRESH_INTERVAL_MS) return false
    this.#refreshes++
    this.#lastRefreshAt = now
    return true
  }

  #trend(queued: number, decodeFps: number, limited: number): 'up' | 'down' | null {
    const last = this.#lastFps ?? this.#userFps
    if (last > this.#userFps) {
      this.#idle = 0
      return 'down'
    }
    if ((queued > 1 && last > limited) || queued > Math.max(1, Math.floor(decodeFps / 2))) {
      this.#idle = 0
      return 'down'
    }
    if (queued > 1) {
      this.#idle = 0
      return null
    }
    this.#idle++
    return this.#idle > IDLE_TICKS && last + STEP_UP <= limited ? 'up' : null
  }
}
