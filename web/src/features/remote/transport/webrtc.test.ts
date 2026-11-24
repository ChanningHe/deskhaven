import { create, fromBinary, toBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'
import { describe, expect, it } from 'vitest'

import { startHandshake, type HandshakeEvent } from '../handshake/handshake'
import {
  ClipboardSchema,
  MessageSchema,
  OptionMessageSchema,
  SignedIdSchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { IdPkSchema } from '../proto/gen/rendezvous_pb.js'
import {
  ANSWER_SDP,
  FakePeerConnection,
  OFFER_SDP,
  PEER_FINGERPRINT,
  answerEndpoint,
} from './fake-rtc'
import { FRAG_END, FRAG_MORE, MAX_FRAGMENT_PAYLOAD, MAX_MESSAGE_BYTES, fragment } from './fragment'
import { TransportError } from './transport'
import { WebRtcTransport } from './webrtc'
import { encodeOffer } from './webrtc-sdp'

const ICE: RTCIceServer[] = [{ urls: ['stun:stun.example:3478'] }]

async function offerer() {
  const pc = new FakePeerConnection()
  const transport = await WebRtcTransport.create(() => pc)
  const channel = pc.channel
  if (!channel) throw new Error('no data channel')
  return { pc, channel, transport }
}

async function connected() {
  const o = await offerer()
  await o.transport.accept(answerEndpoint(), ICE)
  o.channel.open()
  await o.transport.opened
  return o
}

function clipboard(size: number): Message {
  return create(MessageSchema, {
    union: { case: 'clipboard', value: create(ClipboardSchema, { content: new Uint8Array(size) }) },
  })
}

async function nextMessage(transport: WebRtcTransport): Promise<IteratorResult<Message>> {
  return transport.messages[Symbol.asyncIterator]().next()
}

describe('WebRtcTransport offer', () => {
  it('opens an ordered bootstrap channel and offers before any gathering', async () => {
    const { pc, channel, transport } = await offerer()
    expect(channel.label).toBe('bootstrap')
    expect(channel.init).toEqual({ ordered: true })
    expect(channel.binaryType).toBe('arraybuffer')
    expect(pc.calls).toEqual(['createDataChannel', 'createOffer'])
    expect(transport.offer).toBe(encodeOffer(OFFER_SDP))
    expect(transport.offer.length).toBeLessThanOrEqual(1024)
  })

  it('leaves candidates out of the offer', async () => {
    const pc = new FakePeerConnection()
    pc.offerSdp = OFFER_SDP + 'a=candidate:1 1 udp 1 10.0.0.1 9 typ host\r\na=end-of-candidates\r\n'
    const transport = await WebRtcTransport.create(() => pc)
    expect(transport.offer).toBe(encodeOffer(OFFER_SDP))
  })

  it('closes the connection when the offer is over budget', async () => {
    const pc = new FakePeerConnection()
    pc.offerSdp = OFFER_SDP + `a=x-pad:${'0'.repeat(1024)}\r\n`
    await expect(WebRtcTransport.create(() => pc)).rejects.toThrow(/exceeds 1024/)
    expect(pc.closed).toBe(true)
  })
})

describe('WebRtcTransport answer', () => {
  it('sets the ICE servers, then the offer, then the answer, and names the peer fingerprint', async () => {
    const { pc, transport } = await offerer()
    expect(await transport.accept(answerEndpoint(), ICE)).toBe(PEER_FINGERPRINT)
    expect(pc.calls.slice(2)).toEqual([
      'setConfiguration',
      'setLocalDescription',
      'setRemoteDescription',
    ])
    expect(pc.configuration).toEqual({ iceServers: ICE })
    expect(pc.localDescription).toEqual({ type: 'offer', sdp: OFFER_SDP })
    expect(pc.remoteDescription).toEqual({ sdp: ANSWER_SDP })
  })

  it('takes the application section fingerprint when there is no session-level one', async () => {
    const { transport } = await offerer()
    const sdp = ANSWER_SDP.replace(`a=fingerprint:${PEER_FINGERPRINT}\r\n`, '').replace(
      'a=setup:active',
      'a=fingerprint:sha-256 0F:0F\r\na=setup:active',
    )
    expect(await transport.accept(answerEndpoint(sdp), ICE)).toBe('sha-256 0F:0F')
  })

  it.each([
    ['a malformed answer', 'webrtc://%%'],
    ['an answer without a fingerprint', answerEndpoint('v=0\r\nm=application 9 x\r\n')],
    [
      'an answer listing a second fingerprint',
      answerEndpoint(
        ANSWER_SDP.replace(
          `a=fingerprint:${PEER_FINGERPRINT}\r\n`,
          `a=fingerprint:${PEER_FINGERPRINT}\r\na=fingerprint:sha-256 EE:EE\r\n`,
        ),
      ),
    ],
  ])('fails the attempt on %s', async (_, answer) => {
    const { pc, transport } = await offerer()
    await expect(transport.accept(answer, ICE)).rejects.toThrow(TransportError)
    await expect(transport.opened).rejects.toThrow(/WebRTC answer refused/)
    expect(pc.closed).toBe(true)
    expect(pc.remoteDescription).toBeNull()
  })
})

describe('WebRtcTransport messages', () => {
  it('splits a large message into 60000-byte fragments', async () => {
    const { channel, transport } = await connected()
    const message = clipboard(150_000)
    transport.send(message)
    expect(channel.sent.map((p) => [p[0], p.length])).toEqual([
      [FRAG_MORE, MAX_FRAGMENT_PAYLOAD + 1],
      [FRAG_MORE, MAX_FRAGMENT_PAYLOAD + 1],
      [FRAG_END, toBinary(MessageSchema, message).length - 2 * MAX_FRAGMENT_PAYLOAD + 1],
    ])
  })

  it('reassembles the fragments the peer sends', async () => {
    const { channel, transport } = await connected()
    const message = clipboard(150_000)
    for (const part of fragment(toBinary(MessageSchema, message))) channel.receive(part)
    const next = await nextMessage(transport)
    expect(next.done).toBe(false)
    expect(toBinary(MessageSchema, next.value as Message)).toEqual(toBinary(MessageSchema, message))
  })

  it('refuses to send before the channel is open', async () => {
    const { transport } = await offerer()
    expect(() => {
      transport.send(clipboard(1))
    }).toThrow(TransportError)
  })

  it('closes on a framing violation', async () => {
    const { pc, channel, transport } = await connected()
    channel.receive(Uint8Array.of(7, 1, 2))
    await expect(nextMessage(transport)).rejects.toThrow(/neither FRAG_END nor FRAG_MORE/)
    expect(pc.closed).toBe(true)
    expect(channel.closed).toBe(true)
  })

  it('closes when a message grows past the relay frame limit', async () => {
    const { pc, channel, transport } = await connected()
    const chunk = new Uint8Array(MAX_FRAGMENT_PAYLOAD + 1)
    chunk[0] = FRAG_MORE
    for (let sent = 0; sent <= MAX_MESSAGE_BYTES; sent += MAX_FRAGMENT_PAYLOAD) {
      channel.receive(chunk)
    }
    await expect(nextMessage(transport)).rejects.toThrow(/exceeds 67108864/)
    expect(pc.closed).toBe(true)
  })

  it.each([
    [
      'the channel closes',
      (o: Awaited<ReturnType<typeof connected>>) => {
        o.channel.remoteClose()
      },
    ],
    [
      'ICE fails',
      (o: Awaited<ReturnType<typeof connected>>) => {
        o.pc.setState('failed')
      },
    ],
  ])('ends the stream with an error when %s', async (_, fail) => {
    const o = await connected()
    fail(o)
    await expect(nextMessage(o.transport)).rejects.toThrow(TransportError)
    expect(o.pc.closed).toBe(true)
  })

  it('fails the open when ICE fails first', async () => {
    const { pc, transport } = await offerer()
    await transport.accept(answerEndpoint(), ICE)
    pc.setState('failed')
    await expect(transport.opened).rejects.toThrow(/WebRTC connection failed/)
  })

  it('ends quietly on a local close', async () => {
    const { pc, channel, transport } = await connected()
    transport.close()
    expect(await nextMessage(transport)).toEqual({ done: true, value: undefined })
    expect(pc.closed).toBe(true)
    expect(channel.closed).toBe(true)
  })
})

describe('WebRtcTransport candidates', () => {
  it('yields gathered candidates as JSON and ends with gathering', async () => {
    const { pc, transport } = await offerer()
    pc.gather('candidate:1 1 udp 2113937151 a.local 57595 typ host')
    pc.gather('')
    pc.gather(null)
    const out: string[] = []
    for await (const c of transport.localCandidates) out.push(c)
    expect(out.map((c) => JSON.parse(c) as unknown)).toEqual([
      {
        candidate: 'candidate:1 1 udp 2113937151 a.local 57595 typ host',
        sdpMid: '0',
        sdpMLineIndex: 0,
      },
    ])
  })

  it('adds the peer candidate as its ICE agent writes it: sdpMid "" and no ufrag', async () => {
    const { pc, transport } = await offerer()
    const text = JSON.stringify({
      candidate: 'candidate:4219973503 1 udp 2130706431 172.17.0.1 36899 typ host',
      sdpMid: '',
      sdpMLineIndex: 0,
      usernameFragment: null,
    })
    expect(await transport.addRemoteCandidate(text)).toBe(true)
    expect(pc.added).toEqual([
      {
        candidate: 'candidate:4219973503 1 udp 2130706431 172.17.0.1 36899 typ host',
        sdpMid: '',
        sdpMLineIndex: 0,
        usernameFragment: null,
      },
    ])
  })

  it('skips a malformed or refused candidate without failing the attempt', async () => {
    const { pc, transport } = await offerer()
    expect(await transport.addRemoteCandidate('not json')).toBe(false)
    expect(await transport.addRemoteCandidate('{"sdpMid":"0"}')).toBe(false)
    pc.rejectCandidates = true
    expect(await transport.addRemoteCandidate('{"candidate":"x","sdpMid":"0"}')).toBe(false)
    expect(pc.closed).toBe(false)
  })
})

describe('WebRTC identity binding', () => {
  function signedPeer(dtlsFingerprint: string) {
    const id = '42'
    const server = nacl.sign.keyPair()
    const device = nacl.sign.keyPair()
    const box = nacl.box.keyPair()
    const idPk = (pk: Uint8Array, extra: object = {}) =>
      toBinary(IdPkSchema, create(IdPkSchema, { id, pk, ...extra }))
    const signedId = create(MessageSchema, {
      union: {
        case: 'signedId',
        value: create(SignedIdSchema, {
          id: nacl.sign(idPk(box.publicKey, { dtlsFingerprint }), device.secretKey),
        }),
      },
    })
    return {
      params: {
        targetId: id,
        serverPk: server.publicKey,
        signedPeerPk: nacl.sign(idPk(device.publicKey), server.secretKey),
        myId: 'admin',
        myName: 'Admin',
        sessionId: 1n,
        decoders: { vp9: true, av1: false, vp8: false, h264: false, h265: false },
        loginOption: create(OptionMessageSchema),
      },
      signedId,
    }
  }

  async function handshakeOver(signed: string) {
    const o = await offerer()
    const remoteFingerprint = await o.transport.accept(answerEndpoint(), ICE)
    o.channel.open()
    const peer = signedPeer(signed)
    const handshake = startHandshake(
      { kind: 'webrtc', transport: o.transport, remoteFingerprint },
      peer.params,
      { startTimer: () => () => undefined },
    )
    for (const part of fragment(toBinary(MessageSchema, peer.signedId))) o.channel.receive(part)
    const events = handshake.events[Symbol.asyncIterator]()
    return { ...o, events }
  }

  it('keys version 0 over the channel when the peer signed the answer fingerprint', async () => {
    const { channel } = await handshakeOver(PEER_FINGERPRINT)
    await expect.poll(() => channel.sent.length).toBe(1)
    const [first = new Uint8Array()] = channel.sent
    const sent = fromBinary(MessageSchema, first.subarray(1))
    expect(sent.union).toMatchObject({ case: 'publicKey', value: { kxVersion: 0 } })
  })

  it('fails closed and drops the connection when the peer signed another fingerprint', async () => {
    const { pc, channel, events } = await handshakeOver('sha-256 00:11')
    const first = (await events.next()).value as HandshakeEvent
    expect(first).toMatchObject({ kind: 'state', state: { kind: 'failed' } })
    expect(channel.sent).toHaveLength(0)
    expect(pc.closed).toBe(true)
  })
})
