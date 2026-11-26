import type { DataChannelLike, PeerConnectionLike } from './webrtc'

/** An offer as Chromium 147 creates it, without candidates. */
export const OFFER_SDP = [
  'v=0',
  'o=- 3905815367653238493 2 IN IP4 127.0.0.1',
  's=-',
  't=0 0',
  'a=group:BUNDLE 0',
  'a=extmap-allow-mixed',
  'a=msid-semantic: WMS',
  'm=application 9 UDP/DTLS/SCTP webrtc-datachannel',
  'c=IN IP4 0.0.0.0',
  'a=ice-ufrag:Ke+M',
  'a=ice-pwd:3vJ5pQ9xT2mW8rY6nB4cH7kL',
  'a=ice-options:trickle',
  'a=fingerprint:sha-256 5E:8E:1C:42:9B:0F:6D:7A:31:C8:22:E4:90:5B:AF:13:7C:D0:88:41:2E:F6:69:B3:05:CA:74:1D:E2:58:9F:30',
  'a=setup:actpass',
  'a=mid:0',
  'a=sctp-port:5000',
  'a=max-message-size:262144',
  '',
].join('\r\n')

export const PEER_FINGERPRINT =
  'sha-256 A1:B2:C3:D4:E5:F6:07:18:29:3A:4B:5C:6D:7E:8F:90:A1:B2:C3:D4:E5:F6:07:18:29:3A:4B:5C:6D:7E:8F:90'

/** The controlled side's answer: fingerprint at session level, as webrtc-rs writes it. */
export const ANSWER_SDP = [
  'v=0',
  'o=- 4105497566482143810 603957402 IN IP4 0.0.0.0',
  's=-',
  't=0 0',
  `a=fingerprint:${PEER_FINGERPRINT}`,
  'a=group:BUNDLE 0',
  'm=application 9 UDP/DTLS/SCTP webrtc-datachannel',
  'c=IN IP4 0.0.0.0',
  'a=setup:active',
  'a=mid:0',
  'a=sendrecv',
  'a=sctp-port:5000',
  'a=ice-ufrag:pQrStUvW',
  'a=ice-pwd:aBcDeFgHiJkLmNoPqRsTuVwXyZ',
  '',
].join('\r\n')

export function answerEndpoint(sdp: string = ANSWER_SDP): string {
  return `webrtc://${btoa(JSON.stringify({ type: 'answer', sdp, ice_policy: 'all' }))}`
}

export class FakeDataChannel implements DataChannelLike {
  binaryType: BinaryType = 'blob'
  readonly sent: Uint8Array[] = []
  closed = false
  onopen: ((event: Event) => unknown) | null = null
  onclose: ((event: Event) => unknown) | null = null
  onerror: ((event: RTCErrorEvent) => unknown) | null = null
  onmessage: ((event: MessageEvent) => unknown) | null = null

  readonly label: string
  readonly init: RTCDataChannelInit

  constructor(label: string, init: RTCDataChannelInit) {
    this.label = label
    this.init = init
  }

  send(data: Uint8Array<ArrayBuffer>): void {
    if (this.closed) throw new Error('InvalidStateError: channel closed')
    this.sent.push(data.slice())
  }

  close(): void {
    this.closed = true
  }

  open(): void {
    this.onopen?.(new Event('open'))
  }

  receive(data: Uint8Array): void {
    this.onmessage?.(new MessageEvent('message', { data: data.slice().buffer }))
  }

  remoteClose(): void {
    this.onclose?.(new Event('close'))
  }
}

export class FakePeerConnection implements PeerConnectionLike {
  readonly calls: string[] = []
  readonly added: RTCIceCandidateInit[] = []
  channel: FakeDataChannel | null = null
  configuration: RTCConfiguration | null = null
  localDescription: RTCSessionDescriptionInit | null = null
  remoteDescription: { sdp: string } | null = null
  connectionState: RTCPeerConnectionState = 'new'
  closed = false
  offerSdp = OFFER_SDP
  rejectCandidates = false
  onicecandidate: ((event: RTCPeerConnectionIceEvent) => unknown) | null = null
  onconnectionstatechange: ((event: Event) => unknown) | null = null

  createDataChannel(label: string, init: RTCDataChannelInit): FakeDataChannel {
    this.calls.push('createDataChannel')
    this.channel = new FakeDataChannel(label, init)
    return this.channel
  }

  createOffer(): Promise<RTCSessionDescriptionInit> {
    this.calls.push('createOffer')
    return Promise.resolve({ type: 'offer', sdp: this.offerSdp })
  }

  setConfiguration(configuration: RTCConfiguration): void {
    this.calls.push('setConfiguration')
    this.configuration = configuration
  }

  setLocalDescription(description: RTCSessionDescriptionInit): Promise<void> {
    this.calls.push('setLocalDescription')
    this.localDescription = description
    return Promise.resolve()
  }

  setRemoteDescription(description: RTCSessionDescriptionInit): Promise<void> {
    this.calls.push('setRemoteDescription')
    this.remoteDescription = { sdp: description.sdp ?? '' }
    return Promise.resolve()
  }

  addIceCandidate(candidate: RTCIceCandidateInit): Promise<void> {
    if (this.rejectCandidates) return Promise.reject(new Error('OperationError'))
    this.added.push(candidate)
    return Promise.resolve()
  }

  // As RTCPeerConnection.close(): the state turns closed without a connectionstatechange event.
  close(): void {
    this.closed = true
    this.connectionState = 'closed'
  }

  gather(text: string | null): void {
    const candidate =
      text === null
        ? null
        : { candidate: text, toJSON: () => ({ candidate: text, sdpMid: '0', sdpMLineIndex: 0 }) }
    this.onicecandidate?.({ candidate } as unknown as RTCPeerConnectionIceEvent)
  }

  setState(state: RTCPeerConnectionState): void {
    this.connectionState = state
    this.onconnectionstatechange?.(new Event('connectionstatechange'))
  }
}
