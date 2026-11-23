export class MemoryStorage {
  readonly data = new Map<string, string>()

  getItem(key: string): string | null {
    return this.data.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value)
  }
}

export function memoryStorage(): Storage {
  return new MemoryStorage() as unknown as Storage
}
