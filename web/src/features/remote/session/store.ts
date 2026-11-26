/** A value that changes over time; fits React's useSyncExternalStore. */
export interface Store<T> {
  get(): T
  subscribe(listener: () => void): () => void
}

export class ValueStore<T> implements Store<T> {
  #value: T
  readonly #listeners = new Set<() => void>()

  constructor(value: T) {
    this.#value = value
  }

  get(): T {
    return this.#value
  }

  set(value: T): void {
    this.#value = value
    for (const listener of [...this.#listeners]) listener()
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener)
    return () => {
      this.#listeners.delete(listener)
    }
  }
}
