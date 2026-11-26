import { HttpError } from '@/lib/http-error'
import type { WebClientCandidates } from '@/api/generated/model'

/**
 * Each local candidate goes out once more after this delay: the server's hop to the controlled
 * side can be UDP, and the peer's ICE agent drops duplicates.
 */
export const ICE_RESEND_DELAY_MS = 400

const GONE = 410

export interface TrickleApi {
  postCandidate(sessionId: string, candidate: string): Promise<void>
  pollCandidates(
    sessionId: string,
    after: number,
    signal: AbortSignal,
  ): Promise<WebClientCandidates>
}

export interface TricklePeer {
  readonly localCandidates: AsyncIterable<string>
  addRemoteCandidate(candidate: string): Promise<boolean>
}

export interface Trickle {
  /** Stops both directions; the data channel is not touched. */
  stop(): void
}

/** A 410 means the server's route is over (closed or expired): that direction stops, nothing else does. */
export function startTrickle(api: TrickleApi, sessionId: string, peer: TricklePeer): Trickle {
  const trickle = new CandidateTrickle(api, sessionId, peer)
  void trickle.send()
  void trickle.poll()
  return trickle
}

class CandidateTrickle implements Trickle {
  readonly #api: TrickleApi
  readonly #sessionId: string
  readonly #peer: TricklePeer
  readonly #abort = new AbortController()
  readonly #resends = new Set<ReturnType<typeof setTimeout>>()
  #stopped = false
  #sendGone = false
  // Posts go out one at a time, in gathering order.
  #posting: Promise<void> = Promise.resolve()

  constructor(api: TrickleApi, sessionId: string, peer: TricklePeer) {
    this.#api = api
    this.#sessionId = sessionId
    this.#peer = peer
  }

  stop(): void {
    if (this.#stopped) return
    this.#stopped = true
    this.#abort.abort()
    for (const timer of this.#resends) clearTimeout(timer)
    this.#resends.clear()
  }

  async send(): Promise<void> {
    for await (const candidate of this.#peer.localCandidates) {
      if (this.#stopped || this.#sendGone) return
      this.#post(candidate)
      const timer = setTimeout(() => {
        this.#resends.delete(timer)
        this.#post(candidate)
      }, ICE_RESEND_DELAY_MS)
      this.#resends.add(timer)
    }
  }

  async poll(): Promise<void> {
    let after = 0
    const signal = this.#abort.signal
    while (!signal.aborted) {
      let page: WebClientCandidates
      try {
        page = await this.#api.pollCandidates(this.#sessionId, after, signal)
      } catch {
        // 410 (route over), a refusal or the network: the peer's candidates stop here, and
        // ICE goes on with those it has. Retrying would only delay the relay fallback.
        return
      }
      for (const candidate of page.candidates) await this.#peer.addRemoteCandidate(candidate)
      after = page.next
    }
  }

  #post(candidate: string): void {
    this.#posting = this.#posting.then(async () => {
      if (this.#stopped || this.#sendGone) return
      try {
        await this.#api.postCandidate(this.#sessionId, candidate)
      } catch (error) {
        // Any other refusal (invalid, over budget, network) costs only this candidate's path.
        if (error instanceof HttpError && error.status === GONE) this.#sendGone = true
      }
    })
  }
}
