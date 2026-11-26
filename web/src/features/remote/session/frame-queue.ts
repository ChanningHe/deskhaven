/**
 * Decoded frames waiting for the renderer. Only the newest matter for a live screen, so the
 * queue keeps at most this many and closes older ones: an undrawn frame holds decoder memory.
 */
export const MAX_WAITING_FRAMES = 2

/** Single-consumer queue of frames; the consumer closes every frame it takes. */
export class FrameQueue implements AsyncIterable<VideoFrame> {
  readonly #frames: VideoFrame[] = []
  #wake: (() => void) | null = null
  #ended = false

  push(frame: VideoFrame): void {
    if (this.#ended) {
      frame.close()
      return
    }
    this.#frames.push(frame)
    while (this.#frames.length > MAX_WAITING_FRAMES) this.#frames.shift()?.close()
    this.#notify()
  }

  end(): void {
    if (this.#ended) return
    this.#ended = true
    for (const frame of this.#frames.splice(0)) frame.close()
    this.#notify()
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<VideoFrame, void, undefined> {
    for (;;) {
      const frame = this.#frames.shift()
      if (frame) {
        yield frame
        continue
      }
      if (this.#ended) return
      await new Promise<void>((resolve) => {
        this.#wake = resolve
      })
    }
  }

  #notify(): void {
    const wake = this.#wake
    this.#wake = null
    wake?.()
  }
}
