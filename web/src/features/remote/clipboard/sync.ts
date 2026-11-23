import { utf8 } from '../bytes'

export const AUTO_LIMIT_BYTES = 1 << 20

export interface SyncConditions {
  active: boolean
  clipboard: boolean
  clipboardSync: boolean
  viewOnly: boolean
}

export function syncAllowed(c: SyncConditions): boolean {
  return c.active && c.clipboard && c.clipboardSync && !c.viewOnly
}

function normalized(text: string): string {
  return text.replace(/\r\n/g, '\n')
}

function fits(text: string): boolean {
  return utf8(text).length <= AUTO_LIMIT_BYTES
}

/** Text copied before the session must never reach the peer: the first read is a baseline. */
export class ClipboardSync {
  #local: string | null = null
  #peer: string | null = null
  #pending: string | null = null
  #writes = 0

  baseline(text: string | null): void {
    this.#local = text === null ? null : normalized(text)
  }

  readStarts(): number {
    return this.#writes
  }

  /** Reads started before a write may return the old text. */
  writeStarts(): void {
    this.#writes++
  }

  local(text: string, started: number): string | null {
    if (started !== this.#writes) return null
    const seen = normalized(text)
    if (this.#local === null) {
      this.#local = seen
      return null
    }
    if (seen === this.#local) return null
    this.#local = seen
    this.#pending = null
    if (seen === '' || seen === this.#peer || !fits(text)) return null
    this.#peer = seen
    return text
  }

  remote(text: string): string | null {
    const seen = normalized(text)
    this.#peer = seen
    this.#pending = seen === this.#local || !fits(text) ? null : text
    return this.#pending
  }

  wrote(text: string): void {
    this.#local = normalized(text)
    if (this.#pending === text) this.#pending = null
  }

  takePending(): string | null {
    const text = this.#pending
    this.#pending = null
    return text
  }

  dropPending(): void {
    this.#pending = null
  }
}
