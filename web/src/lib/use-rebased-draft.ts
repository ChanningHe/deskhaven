import { useState } from 'react'

interface DraftState<T> {
  server: T
  /** The server copy the draft was seeded from (field by field). */
  baseline: T
  draft: T
}

export interface RebasedDraft<T> {
  draft: T
  changed: (keyof T)[]
  conflicts: (keyof T)[]
  set: (change: Partial<T>) => void
  /** Only the sent fields are rebased; what the user typed meanwhile stays in the draft. */
  reset: (saved: T, sent: Partial<T>) => void
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

function keysOf<T extends object>(value: T): (keyof T)[] {
  return Object.keys(value) as (keyof T)[]
}

function isUntouched<T extends object>(state: DraftState<T>, key: keyof T): boolean {
  return same(state.draft[key], state.baseline[key])
}

/** A field typed back to its baseline is untouched again; an untouched key the server dropped goes. */
function rebase<T extends object>(state: DraftState<T>, server: T): DraftState<T> {
  const baseline = { ...state.baseline }
  const draft = { ...state.draft }
  const keys = new Set([...keysOf(server), ...keysOf(state.baseline)])
  for (const key of keys) {
    if (!isUntouched(state, key)) continue
    if (key in server) {
      baseline[key] = server[key]
      draft[key] = server[key]
    } else {
      // Fresh copies: removing the key changes nothing the caller holds.
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
      delete baseline[key]
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
      delete draft[key]
    }
  }
  return { server, baseline, draft }
}

function isStale<T extends object>(state: DraftState<T>, server: T): boolean {
  if (!same(state.server, server)) return true
  return keysOf(server).some(
    (key) => isUntouched(state, key) && !same(state.baseline[key], server[key]),
  )
}

/**
 * Diffed against the baseline it was seeded from, never the live copy: saving an unrelated field
 * cannot write a stale value back, and an edited field the server changed shows in `conflicts`.
 */
export function useRebasedDraft<T extends object>(server: T): RebasedDraft<T> {
  const [state, setState] = useState<DraftState<T>>(() => ({
    server,
    baseline: server,
    draft: server,
  }))
  let current = state
  if (isStale(state, server)) {
    // Adjusting state while rendering is the documented way to derive it from a prop.
    current = rebase(state, server)
    setState(current)
  }
  const changed = keysOf(current.draft).filter((key) => !isUntouched(current, key))
  const conflicts = changed.filter((key) => !same(current.server[key], current.baseline[key]))
  return {
    draft: current.draft,
    changed,
    conflicts,
    set: (change) => {
      setState((previous) => ({ ...previous, draft: { ...previous.draft, ...change } }))
    },
    reset: (saved, sent) => {
      setState((previous) => {
        const baseline = { ...previous.baseline }
        const draft = { ...previous.draft }
        for (const key of keysOf(sent)) {
          baseline[key] = saved[key]
          // Typed further while the save was in flight: keep the newer text.
          if (same(previous.draft[key], sent[key])) draft[key] = saved[key]
        }
        return rebase({ server: saved, baseline, draft }, saved)
      })
    },
  }
}
