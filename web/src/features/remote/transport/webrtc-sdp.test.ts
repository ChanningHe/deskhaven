import { describe, expect, it } from 'vitest'

import {
  MAX_OFFER_BYTES,
  decodeAnswer,
  encodeOffer,
  sdpFingerprint,
  trickleSdp,
} from './webrtc-sdp'

// A Firefox 148 offer as createOffer returns it, plus candidate lines to strip.
const FIREFOX_OFFER = [
  'v=0',
  'o=mozilla...THIS_IS_SDPARTA-99.0 4372513290474452352 0 IN IP4 0.0.0.0',
  's=-',
  't=0 0',
  'a=sendrecv',
  'a=fingerprint:sha-256 5E:8E:1C:42:9B:0F:6D:7A:31:C8:22:E4:90:5B:AF:13:7C:D0:88:41:2E:F6:69:B3:05:CA:74:1D:E2:58:9F:30',
  'a=group:BUNDLE 0',
  'a=ice-options:trickle',
  'a=msid-semantic:WMS *',
  'm=application 9 UDP/DTLS/SCTP webrtc-datachannel',
  'c=IN IP4 0.0.0.0',
  'a=candidate:0 1 UDP 2122252543 2a0b4d6e-8c01-4f3e-9d2a-7b5c1e0f3a42.local 57595 typ host',
  'a=sendrecv',
  'a=end-of-candidates',
  'a=ice-pwd:9d3c51f0e2b84a7f6c19d0e5a3b27f48',
  'a=ice-ufrag:4f2a9c1e',
  'a=mid:0',
  'a=setup:actpass',
  'a=sctp-port:5000',
  'a=max-message-size:1073741823',
  '',
].join('\r\n')

function envelope(endpoint: string): unknown {
  return JSON.parse(atob(endpoint.slice('webrtc://'.length)))
}

function answerEndpoint(json: object): string {
  return `webrtc://${btoa(JSON.stringify(json))}`
}

describe('offer endpoint', () => {
  it('drops candidate and end-of-candidates lines, keeping every other line in order', () => {
    const sdp = trickleSdp(FIREFOX_OFFER)
    expect(sdp).not.toMatch(/a=candidate|a=end-of-candidates/)
    expect(sdp.split('\r\n')).toEqual(
      FIREFOX_OFFER.split('\r\n').filter((l) => !/^a=(candidate:|end-of-candidates)/.test(l)),
    )
  })

  it('is webrtc:// and base64 of {type, sdp, ice_policy: all}, like the client offerer', () => {
    const endpoint = encodeOffer(FIREFOX_OFFER)
    expect(endpoint.startsWith('webrtc://')).toBe(true)
    expect(envelope(endpoint)).toEqual({
      type: 'offer',
      sdp: trickleSdp(FIREFOX_OFFER),
      ice_policy: 'all',
    })
  })

  it('fits the 1024-byte UDP budget for a real browser offer', () => {
    expect(encodeOffer(FIREFOX_OFFER).length).toBeLessThanOrEqual(MAX_OFFER_BYTES)
  })

  it('refuses an offer over the budget instead of sending it', () => {
    const padded = FIREFOX_OFFER + `a=x-pad:${'0'.repeat(MAX_OFFER_BYTES)}\r\n`
    expect(() => encodeOffer(padded)).toThrow(/exceeds 1024/)
  })
})

describe('answer endpoint', () => {
  it('decodes {type: answer, sdp, ice_policy}', () => {
    const sdp = 'v=0\r\na=fingerprint:sha-256 AA:BB\r\n'
    expect(decodeAnswer(answerEndpoint({ type: 'answer', sdp, ice_policy: 'all' }))).toBe(sdp)
    expect(decodeAnswer(answerEndpoint({ type: 'answer', sdp }))).toBe(sdp)
  })

  it.each([
    ['no scheme', btoa(JSON.stringify({ type: 'answer', sdp: 'v=0' }))],
    ['bad base64', 'webrtc://not-base64!'],
    ['not JSON', `webrtc://${btoa('v=0')}`],
    ['an offer', answerEndpoint({ type: 'offer', sdp: 'v=0' })],
    ['no sdp', answerEndpoint({ type: 'answer' })],
    ['a non-string sdp', answerEndpoint({ type: 'answer', sdp: 7 })],
  ])('rejects %s', (_, endpoint) => {
    expect(() => decodeAnswer(endpoint)).toThrow()
  })
})

describe('sdpFingerprint', () => {
  const sdp = (...lines: string[]) => ['v=0', ...lines, ''].join('\r\n')
  const APP = 'm=application 9 UDP/DTLS/SCTP webrtc-datachannel'

  it('takes the one fingerprint, at session level or in a media section', () => {
    expect(sdpFingerprint(sdp('a=fingerprint:sha-256 AA:AA', APP))).toBe('sha-256 AA:AA')
    expect(sdpFingerprint(sdp(APP, 'a=fingerprint:sha-256 BB:BB'))).toBe('sha-256 BB:BB')
  })

  it('accepts the same fingerprint repeated at every level', () => {
    const repeated = sdp('a=fingerprint:sha-256 AA:AA', APP, 'a=fingerprint:sha-256 AA:AA')
    expect(sdpFingerprint(repeated)).toBe('sha-256 AA:AA')
  })

  // Firefox completes DTLS when any listed fingerprint matches, so a signalling party could
  // list its own next to the device's; the binding only holds with a single value.
  it('is null when the fingerprints disagree anywhere in the description', () => {
    const twoAtSession = sdp('a=fingerprint:sha-256 AA:AA', 'a=fingerprint:sha-256 EE:EE', APP)
    const sessionVsMedia = sdp('a=fingerprint:sha-256 AA:AA', APP, 'a=fingerprint:sha-256 BB:BB')
    const otherMedia = sdp(
      'm=audio 9 UDP/TLS/RTP/SAVPF 111',
      'a=fingerprint:sha-256 CC:CC',
      APP,
      'a=fingerprint:sha-256 BB:BB',
    )
    expect(sdpFingerprint(twoAtSession)).toBeNull()
    expect(sdpFingerprint(sessionVsMedia)).toBeNull()
    expect(sdpFingerprint(otherMedia)).toBeNull()
  })

  it('is null without one', () => {
    expect(sdpFingerprint('v=0\r\nm=application 9 x\r\na=mid:0\r\n')).toBeNull()
  })
})
