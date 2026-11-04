import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { useRebasedDraft } from './use-rebased-draft'

interface Form {
  note: string
  group: number | null
}

const BASE: Form = { note: 'a', group: null }

function setup(initial: Form = BASE) {
  return renderHook(({ server }) => useRebasedDraft(server), { initialProps: { server: initial } })
}

describe('useRebasedDraft', () => {
  it('lets untouched fields follow the server and keeps edited ones, flagging the conflict', () => {
    const { result, rerender } = setup()
    act(() => {
      result.current.set({ note: 'mine' })
    })

    rerender({ server: { note: 'theirs', group: 3 } })

    expect(result.current.draft).toEqual({ note: 'mine', group: 3 })
    expect(result.current.changed).toEqual(['note'])
    expect(result.current.conflicts).toEqual(['note'])
  })

  it('follows the server again for a field typed back to its baseline', () => {
    const { result, rerender } = setup()
    act(() => {
      result.current.set({ note: 'b' })
    })
    // The poll brings c; the edited field is skipped, its baseline stays a.
    rerender({ server: { note: 'c', group: null } })
    expect(result.current.draft.note).toBe('b')

    act(() => {
      result.current.set({ note: 'a' })
    })

    // Untouched now, so it shows what the server has, with no later poll needed.
    expect(result.current.draft.note).toBe('c')
    expect(result.current.changed).toEqual([])
  })

  it('rebases only the sent fields after a save and keeps what was typed meanwhile', () => {
    const { result, rerender } = setup()
    act(() => {
      result.current.set({ note: 'sent' })
    })
    // The user keeps typing while the save is in flight, and edits another field.
    act(() => {
      result.current.set({ note: 'sent, and more', group: 7 })
    })

    act(() => {
      result.current.reset({ note: 'sent', group: null }, { note: 'sent' })
    })
    rerender({ server: { note: 'sent', group: null } })

    expect(result.current.draft).toEqual({ note: 'sent, and more', group: 7 })
    expect(result.current.changed).toEqual(['note', 'group'])
  })

  it('adopts the saved copy for sent fields the user did not touch again', () => {
    const { result, rerender } = setup()
    act(() => {
      result.current.set({ note: 'sent' })
    })

    act(() => {
      result.current.reset({ note: 'sent (normalised)', group: null }, { note: 'sent' })
    })
    rerender({ server: { note: 'sent (normalised)', group: null } })

    expect(result.current.draft.note).toBe('sent (normalised)')
    expect(result.current.changed).toEqual([])
  })

  it('drops an untouched key the server copy no longer has and keeps an edited one', () => {
    const initial: Record<string, string | undefined> = { a: '1', b: '2' }
    const { result, rerender } = renderHook(({ server }) => useRebasedDraft(server), {
      initialProps: { server: initial },
    })
    act(() => {
      result.current.set({ b: 'mine', c: 'new' })
    })

    rerender({ server: {} })

    expect(result.current.draft.a).toBeUndefined()
    expect(result.current.draft).toMatchObject({ b: 'mine', c: 'new' })
    expect(result.current.changed.sort()).toEqual(['b', 'c'])
    expect(result.current.conflicts).toEqual(['b'])
  })
})
