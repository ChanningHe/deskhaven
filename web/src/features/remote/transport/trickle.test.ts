import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import type { WebClientCandidates } from '@/api/generated/model'

import { AsyncQueue } from '../async-queue'
import { ICE_RESEND_DELAY_MS, startTrickle, type TrickleApi } from './trickle'

const SESSION = 'sess-1'

/** A poll the test answers by hand, one at a time. */
class Polls {
  readonly calls: { after: number; signal: AbortSignal }[] = []
  #pending: { resolve: (page: WebClientCandidates) => void; reject: (e: Error) => void }[] = []

  poll = (_: string, after: number, signal: AbortSignal): Promise<WebClientCandidates> => {
    this.calls.push({ after, signal })
    return new Promise((resolve, reject) => {
      this.#pending.push({ resolve, reject })
      signal.addEventListener('abort', () => {
        reject(new DOMException('aborted', 'AbortError'))
      })
    })
  }

  answer(page: WebClientCandidates): void {
    this.#pending.shift()?.resolve(page)
  }

  fail(error: Error): void {
    this.#pending.shift()?.reject(error)
  }
}

function setup() {
  const local = new AsyncQueue<string>()
  const added: string[] = []
  const polls = new Polls()
  const postCandidate = vi.fn<TrickleApi['postCandidate']>(() => Promise.resolve())
  const api: TrickleApi = { postCandidate, pollCandidates: polls.poll }
  const peer = {
    localCandidates: local,
    addRemoteCandidate: (c: string) => {
      added.push(c)
      return Promise.resolve(true)
    },
  }
  const trickle = startTrickle(api, SESSION, peer)
  return { local, added, polls, postCandidate, trickle }
}

const gone = () => new ApiError(410, { code: 'session_closed' })

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('trickle: our candidates', () => {
  it('posts each candidate as it comes, then once more after 400 ms', async () => {
    const t = setup()
    t.local.push('a')
    t.local.push('b')
    await vi.advanceTimersByTimeAsync(0)
    expect(t.postCandidate.mock.calls).toEqual([
      [SESSION, 'a'],
      [SESSION, 'b'],
    ])
    await vi.advanceTimersByTimeAsync(ICE_RESEND_DELAY_MS - 1)
    expect(t.postCandidate).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(1)
    expect(t.postCandidate.mock.calls.slice(2)).toEqual([
      [SESSION, 'a'],
      [SESSION, 'b'],
    ])
    await vi.advanceTimersByTimeAsync(10 * ICE_RESEND_DELAY_MS)
    expect(t.postCandidate).toHaveBeenCalledTimes(4)
  })

  it('posts candidates gathered before it started, in order', async () => {
    const local = new AsyncQueue<string>()
    local.push('early-1')
    local.push('early-2')
    const postCandidate = vi.fn<TrickleApi['postCandidate']>(() => Promise.resolve())
    startTrickle({ postCandidate, pollCandidates: new Polls().poll }, SESSION, {
      localCandidates: local,
      addRemoteCandidate: () => Promise.resolve(true),
    })
    await vi.advanceTimersByTimeAsync(0)
    expect(postCandidate.mock.calls.map((c) => c[1])).toEqual(['early-1', 'early-2'])
  })

  it('stops posting once the route is gone, resends included', async () => {
    const t = setup()
    t.postCandidate.mockRejectedValueOnce(gone())
    t.local.push('a')
    await vi.advanceTimersByTimeAsync(0)
    t.local.push('b')
    await vi.advanceTimersByTimeAsync(ICE_RESEND_DELAY_MS)
    expect(t.postCandidate.mock.calls.map((c) => c[1])).toEqual(['a'])
  })

  it('goes on after a candidate the server refuses', async () => {
    const t = setup()
    t.postCandidate.mockRejectedValueOnce(new ApiError(400, { code: 'invalid_candidate' }))
    t.local.push('a')
    t.local.push('b')
    await vi.advanceTimersByTimeAsync(0)
    expect(t.postCandidate.mock.calls.map((c) => c[1])).toEqual(['a', 'b'])
  })

  it('sends nothing, not even pending resends, after stop', async () => {
    const t = setup()
    t.local.push('a')
    await vi.advanceTimersByTimeAsync(0)
    t.trickle.stop()
    t.local.push('b')
    await vi.advanceTimersByTimeAsync(ICE_RESEND_DELAY_MS)
    expect(t.postCandidate.mock.calls.map((c) => c[1])).toEqual(['a'])
  })
})

describe('trickle: the peer candidates', () => {
  it('long-polls from the last index on and adds each candidate', async () => {
    const t = setup()
    await vi.advanceTimersByTimeAsync(0)
    expect(t.polls.calls.map((c) => c.after)).toEqual([0])
    t.polls.answer({ candidates: ['x', 'y'], next: 2 })
    await vi.advanceTimersByTimeAsync(0)
    t.polls.answer({ candidates: [], next: 2 })
    await vi.advanceTimersByTimeAsync(0)
    expect(t.added).toEqual(['x', 'y'])
    expect(t.polls.calls.map((c) => c.after)).toEqual([0, 2, 2])
  })

  it('stops polling on 410 and leaves the rest alone', async () => {
    const t = setup()
    await vi.advanceTimersByTimeAsync(0)
    t.polls.fail(gone())
    await vi.advanceTimersByTimeAsync(60_000)
    expect(t.polls.calls).toHaveLength(1)
    t.local.push('a')
    await vi.advanceTimersByTimeAsync(0)
    expect(t.postCandidate).toHaveBeenCalledWith(SESSION, 'a')
  })

  it('aborts the poll in flight on stop', async () => {
    const t = setup()
    await vi.advanceTimersByTimeAsync(0)
    t.trickle.stop()
    expect(t.polls.calls[0]?.signal.aborted).toBe(true)
    await vi.advanceTimersByTimeAsync(0)
    expect(t.polls.calls).toHaveLength(1)
  })
})
