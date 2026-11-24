export const DEFAULT_FPS = 30
export const INACTIVE_TICKS = 15
export const REFRESH_INTERVAL_MS = 10_000
export const MAX_REFRESHES = 20
/** Chunks waiting for the decoder to take them: it is no longer keeping up. */
export const BACKLOG = 2
export const MIN_AUTO_FPS = 5

const WARM_UP_FRAMES = 3
const RATE_EVERY = 10
const RATE_WINDOW = 30
const IDLE_TICKS = 3
const STEP_UP = 3
// A decoder's configure is asynchronous: chunks queue behind it and a cut takes a round trip to
// show, so for this many ticks the backlog says nothing new.
export const SETTLE_TICKS = 2
const COOL_TICKS = 2

export interface FpsGovernorOptions {
  userFps: number
}

export interface FpsTick {
  fps: number
  backlog: number
}

// rustdesk 1.5.1 src/client/io_loop.rs:1557-1655, decode rate src/client.rs:4164-4193. Unlike
// there, the decode rate does not cap the fps: a WebCodecs decoder pipelines frames, so the time
// one frame spends in it is latency, not throughput. Only a backlog says it is falling behind.
export class FpsGovernor {
  #userFps: number
  #userChanged = false
  #fps: number
  #warmUp = 0
  #count = 0
  #totalMs = 0
  #decodeFps: number | null = null
  #idle = 0
  #hold = 0
  #inactive = 0
  #refreshes = 0
  #lastRefreshAt = 0

  constructor(options: FpsGovernorOptions) {
    this.#userFps = options.userFps
    this.#fps = options.userFps
  }

  get userFps(): number {
    return this.#userFps
  }

  /** Diagnostics only. */
  get decodeFps(): number | null {
    return this.#decodeFps
  }

  setUserFps(fps: number): void {
    if (fps === this.#userFps) return
    this.#userFps = fps
    this.#userChanged = true
  }

  /** A new decoder's first frames take its setup too: they are skipped as the first one's are. */
  decoderStarted(): void {
    this.#warmUp = 0
    this.#hold = Math.max(this.#hold, SETTLE_TICKS)
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

  tick({ fps, backlog }: FpsTick): number | null {
    this.#inactive = fps === 0 ? this.#inactive + 1 : 0
    const next = this.#next(backlog)
    if (next === this.#fps) return null
    this.#fps = next
    return next
  }

  #next(backlog: number): number {
    const held = this.#hold > 0
    if (held) this.#hold--
    if (this.#userChanged) {
      this.#userChanged = false
      this.#idle = 0
      return this.#userFps
    }
    // A screen that stopped changing sends nothing, so what still waits says nothing new.
    if (backlog >= BACKLOG && this.#inactive < INACTIVE_TICKS) {
      this.#idle = 0
      if (held) return this.#fps
      this.#hold = COOL_TICKS
      return Math.min(this.#fps, Math.max(MIN_AUTO_FPS, Math.floor(this.#fps / 2)))
    }
    this.#idle++
    if (this.#idle <= IDLE_TICKS) return this.#fps
    return Math.min(this.#userFps, this.#fps + STEP_UP)
  }

  mayRefresh(now: number): boolean {
    if (this.#refreshes >= MAX_REFRESHES) return false
    if (this.#refreshes > 0 && now - this.#lastRefreshAt <= REFRESH_INTERVAL_MS) return false
    this.#refreshes++
    this.#lastRefreshAt = now
    return true
  }
}
