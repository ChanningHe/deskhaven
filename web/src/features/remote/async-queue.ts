/** Unbounded single-consumer queue that hands items to one `for await` loop in push order. */
export class AsyncQueue<T> {
  readonly #items: T[] = []
  #wake: (() => void) | null = null
  #ended = false
  #error: Error | null = null

  /** Drops the item once the queue has ended. */
  push(item: T): void {
    if (this.#ended) return
    this.#items.push(item)
    this.#notify()
  }

  /** The consumer drains what is queued, then its loop ends (or throws `error`). */
  end(error?: Error): void {
    if (this.#ended) return
    this.#ended = true
    this.#error = error ?? null
    this.#notify()
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<T, void, undefined> {
    for (;;) {
      if (this.#items.length > 0) {
        yield this.#items.shift() as T
        continue
      }
      if (this.#ended) {
        if (this.#error) throw this.#error
        return
      }
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
