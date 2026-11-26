/** `webrtc://` + base64 of the JSON session description `{type, sdp, ice_policy?}`. */

const SCHEME = 'webrtc://'

/**
 * A device registered over UDP receives the offer inside one PunchHole datagram, and one that needs
 * IP fragmentation is dropped on paths that discard fragments. The relay request rides in the same
 * PunchHole, so an oversize offer would cost the relay too: it is not sent at all.
 */
export const MAX_OFFER_BYTES = 1024

/** Candidates trickle separately, and `end-of-candidates` would tell the peer to stop waiting. */
export function trickleSdp(sdp: string): string {
  return sdpLines(sdp)
    .filter((line) => !line.startsWith('a=candidate:') && !line.startsWith('a=end-of-candidates'))
    .map((line) => `${line}\r\n`)
    .join('')
}

/** `ice_policy: all` says our ICE is not relay-only, so the peer answers with full ICE even under force_relay. */
export function encodeOffer(sdp: string): string {
  const json = JSON.stringify({ type: 'offer', sdp: trickleSdp(sdp), ice_policy: 'all' })
  const endpoint = SCHEME + toBase64(new TextEncoder().encode(json))
  if (endpoint.length > MAX_OFFER_BYTES) {
    throw new Error(`WebRTC offer of ${endpoint.length} bytes exceeds ${MAX_OFFER_BYTES}`)
  }
  return endpoint
}

export function decodeAnswer(endpoint: string): string {
  if (!endpoint.startsWith(SCHEME)) throw new Error('WebRTC answer is not a webrtc:// endpoint')
  let envelope: unknown
  try {
    const bytes = fromBase64(endpoint.slice(SCHEME.length))
    envelope = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  } catch {
    throw new Error('WebRTC answer is not base64 of a JSON session description')
  }
  if (typeof envelope !== 'object' || envelope === null) {
    throw new Error('WebRTC answer is not a session description')
  }
  const { type, sdp } = envelope as Record<string, unknown>
  if (type !== 'answer' || typeof sdp !== 'string') {
    throw new Error('WebRTC answer is not an answer session description')
  }
  return sdp
}

/**
 * Null unless the description carries exactly one fingerprint: Firefox completes DTLS when any
 * listed one matches, so a second would let a signalling party bind its own certificate while the
 * signed value still compares equal.
 */
export function sdpFingerprint(sdp: string): string | null {
  const values = new Set(
    sdpLines(sdp)
      .filter((line) => line.startsWith('a=fingerprint:'))
      .map((line) => line.slice('a=fingerprint:'.length)),
  )
  if (values.size !== 1) return null
  const [value] = values
  return value ?? null
}

function sdpLines(sdp: string): string[] {
  return sdp.split('\n').flatMap((line) => {
    const trimmed = line.endsWith('\r') ? line.slice(0, -1) : line
    return trimmed === '' ? [] : [trimmed]
  })
}

function toBase64(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
}

function fromBase64(text: string): Uint8Array {
  const binary = atob(text)
  return Uint8Array.from(binary, (c) => c.charCodeAt(0))
}
