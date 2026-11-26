import { CONNECT_TIMEOUT_MS } from '../handshake/handshake'
import { TransportError, type KeyedTransport } from '../transport/transport'

/** The relay connects an order of magnitude faster than ICE + DTLS + SCTP. */
export const RELAY_FALLBACK_DELAY_MS = 2500

/** The client's own bound on the data channel opening. */
export const WEBRTC_CONNECT_TIMEOUT_MS = CONNECT_TIMEOUT_MS

export type WebRtcOutcome = 'pending' | { failed: unknown }

export type RaceResult<W, R> =
  { winner: 'webrtc'; value: W } | { winner: 'relay'; value: R; webrtc: WebRtcOutcome }

/**
 * WebRTC wins as soon as it is ready; a relay that is ready first is held for `graceMs` and
 * taken when WebRTC fails or the time is up. The caller closes the loser.
 */
export function preferWebRtc<W, R>(
  webrtc: Promise<W>,
  relay: Promise<R>,
  graceMs: number = RELAY_FALLBACK_DELAY_MS,
): Promise<RaceResult<W, R>> {
  return new Promise((resolve, reject) => {
    let settled = false
    let webrtcError: { error: unknown } | null = null
    let relayError: { error: unknown } | null = null
    let held: { value: R } | null = null
    let grace: ReturnType<typeof setTimeout> | null = null

    const finish = (result: RaceResult<W, R>) => {
      if (settled) return
      settled = true
      if (grace !== null) clearTimeout(grace)
      resolve(result)
    }
    const bothFailed = () => {
      if (settled || !webrtcError || !relayError) return
      settled = true
      reject(
        new Error(
          `WebRTC failed: ${errorText(webrtcError.error)}; relay failed: ${errorText(relayError.error)}`,
        ),
      )
    }

    webrtc.then(
      (value) => {
        finish({ winner: 'webrtc', value })
      },
      (error: unknown) => {
        webrtcError = { error }
        if (held) finish({ winner: 'relay', value: held.value, webrtc: { failed: error } })
        else bothFailed()
      },
    )
    relay.then(
      (value) => {
        if (settled) return
        if (webrtcError) {
          finish({ winner: 'relay', value, webrtc: { failed: webrtcError.error } })
          return
        }
        held = { value }
        grace = setTimeout(() => {
          finish({ winner: 'relay', value, webrtc: 'pending' })
        }, graceMs)
      },
      (error: unknown) => {
        relayError = { error }
        bothFailed()
      },
    )
  })
}

export function withTimeout<T>(promise: Promise<T>, ms: number, reason: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new TransportError(reason))
    }, ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(error instanceof Error ? error : new Error(String(error)))
      },
    )
  })
}

export async function paired(relay: KeyedTransport): Promise<KeyedTransport> {
  const frames = relay.messages[Symbol.asyncIterator]()
  const first = await frames.next()
  if (first.done === true) throw new TransportError('relay connection closed before pairing')
  const head = first.value
  async function* replay() {
    yield head
    for (;;) {
      const next = await frames.next()
      if (next.done === true) return
      yield next.value
    }
  }
  return {
    messages: replay(),
    send: (message) => {
      relay.send(message)
    },
    close: () => {
      relay.close()
    },
    startEncryption: (cipher) => {
      relay.startEncryption(cipher)
    },
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
