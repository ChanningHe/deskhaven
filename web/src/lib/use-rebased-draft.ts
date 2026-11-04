import { useState } from 'react'

interface DraftState<T> {
  /** The last server copy seen. */
  server: T
  /** The server copy the draft was seeded from (field by field). */
  baseline: T
  draft: T
}

export interface RebasedDraft<T> {
  draft: T
  /** Fields the user edited: they differ from the baseline the draft was seeded from. */
  changed: (keyof T)[]
  /** Edited fields whose server value has changed since the edit began. */
  conflicts: (keyof T)[]
  set: (change: Partial<T>) => void
  /**
   * A save succeeded: `saved` is the server's copy and `sent` the values it was sent with.
   * Only the sent fields are rebased; what the user typed meanwhile (in other fields, or
   * further in a sent one) stays in the draft.
   */
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

/**
 * Untouched fields follow the server copy, edited ones keep the user's text. A field the
 * user edited and then typed back to its baseline is untouched again, so it follows too.
 * A key the server copy no longer has (a record of options) is dropped when untouched.
 */
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

/** Whether `state` lags behind `server`: a new copy, or an untouched field out of date. */
function isStale<T extends object>(state: DraftState<T>, server: T): boolean {
  if (!same(state.server, server)) return true
  return keysOf(server).some(
    (key) => isUntouched(state, key) && !same(state.baseline[key], server[key]),
  )
}

/**
 * A form draft over a polled server copy (a lost-update guard). The draft is diffed against
 * the baseline it was seeded from, never against the live server copy: when another admin
 * changes a field the user did not touch, the field follows the server, and saving an
 * unrelated field cannot write the stale value back. A field the user did edit keeps their
 * text and shows up in `conflicts` when the server changed it meanwhile.
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
