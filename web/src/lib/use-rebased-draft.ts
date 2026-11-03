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
  /** Adopts `saved` (the server's copy after a save) as baseline and draft. */
  reset: (saved: T) => void
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

function keysOf<T extends object>(value: T): (keyof T)[] {
  return Object.keys(value) as (keyof T)[]
}

/** Untouched fields follow the new server copy; edited ones keep the user's text. */
function rebase<T extends object>(state: DraftState<T>, server: T): DraftState<T> {
  const baseline = { ...state.baseline }
  const draft = { ...state.draft }
  for (const key of keysOf(server)) {
    if (!same(state.draft[key], state.baseline[key])) continue
    baseline[key] = server[key]
    draft[key] = server[key]
  }
  return { server, baseline, draft }
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
  if (!same(state.server, server)) {
    // Adjusting state while rendering is the documented way to derive it from a prop.
    current = rebase(state, server)
    setState(current)
  }
  const changed = keysOf(current.draft).filter((k) => !same(current.draft[k], current.baseline[k]))
  const conflicts = changed.filter((k) => !same(current.server[k], current.baseline[k]))
  return {
    draft: current.draft,
    changed,
    conflicts,
    set: (change) => {
      setState((previous) => ({ ...previous, draft: { ...previous.draft, ...change } }))
    },
    reset: (saved) => {
      setState({ server: saved, baseline: saved, draft: saved })
    },
  }
}
