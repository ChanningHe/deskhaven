import { create, fromBinary, toBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'
import { describe, expect, it } from 'vitest'

import { FakeSocket, PAIRING_UUID, msg, relayWith } from '../__fixtures__/harness'
import { FrameCipher } from '../crypto/frame-cipher'
import {
  MessageSchema,
  MiscSchema,
  TestDelaySchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { RendezvousMessageSchema } from '../proto/gen/rendezvous_pb.js'
import { MAX_FRAME_BYTES, MAX_QUEUED_BYTES, MAX_QUEUED_FRAMES, relayUrl } from './relay'
import { TransportError } from './transport'

const delay = (time: bigint): Message =>
  msg({ case: 'testDelay', value: create(TestDelaySchema, { time }) })

async function pull(messages: AsyncIterator<Message>): Promise<Message> {
  const next = await messages.next()
  if (next.done === true) throw new Error('messages ended')
  return next.value
}

function opened(): { socket: FakeSocket; relay: ReturnType<typeof relayWith> } {
  const socket = new FakeSocket('wss://console.test/ws/relay')
  const relay = relayWith(socket, 'c2VydmVyLWtleQ==', '987654321')
  socket.open()
  return { socket, relay }
}

describe('relayUrl', () => {
  it('uses the page origin, secure when the page is', () => {
    expect(relayUrl({ protocol: 'https:', host: 'console.example:8443' })).toBe(
      'wss://console.example:8443/ws/relay',
    )
    expect(relayUrl({ protocol: 'http:', host: 'localhost:5173' })).toBe(
      'ws://localhost:5173/ws/relay',
    )
  })
})

describe('RelayTransport', () => {
  it('opens the given URL with binary frames and sends the pairing request first', () => {
    const { socket } = opened()
    expect(socket.binaryType).toBe('arraybuffer')
    expect(socket.sent).toHaveLength(1)
    const first = fromBinary(RendezvousMessageSchema, socket.sent[0] ?? new Uint8Array())
    expect(first.union.case).toBe('requestRelay')
    expect(first.union.value).toMatchObject({
      licenceKey: 'c2VydmVyLWtleQ==',
      uuid: PAIRING_UUID,
      id: '987654321',
    })
  })

  it('carries one message per binary frame in plaintext until keyed', async () => {
    const { socket, relay } = opened()
    socket.receive(toBinary(MessageSchema, delay(7n)))
    const next = await pull(relay.messages[Symbol.asyncIterator]())
    expect(next.union).toMatchObject({ case: 'testDelay', value: { time: 7n } })
    relay.send(delay(8n))
    expect(fromBinary(MessageSchema, socket.sent[1] ?? new Uint8Array()).union).toMatchObject({
      value: { time: 8n },
    })
  })

  it('seals and opens every frame once keyed', async () => {
    const { socket, relay } = opened()
    const key = nacl.randomBytes(32)
    const peer = FrameCipher.v0(key)
    relay.startEncryption(FrameCipher.v0(key))
    relay.send(delay(1n))
    expect(
      fromBinary(MessageSchema, peer.open(socket.sent[1] ?? new Uint8Array())).union,
    ).toMatchObject({ value: { time: 1n } })
    socket.receive(peer.seal(toBinary(MessageSchema, delay(2n))))
    const next = await pull(relay.messages[Symbol.asyncIterator]())
    expect(next.union).toMatchObject({ value: { time: 2n } })
    expect(() => {
      relay.startEncryption(FrameCipher.v0(key))
    }).toThrow()
  })

  it('applies the key from the next pulled frame, even if that frame arrived earlier', async () => {
    const { socket, relay } = opened()
    const key = nacl.randomBytes(32)
    const peer = FrameCipher.v0(key)
    socket.receive(toBinary(MessageSchema, delay(1n)))
    socket.receive(peer.seal(toBinary(MessageSchema, delay(2n))))
    const messages = relay.messages[Symbol.asyncIterator]()
    expect((await pull(messages)).union).toMatchObject({ value: { time: 1n } })
    relay.startEncryption(FrameCipher.v0(key))
    expect((await pull(messages)).union).toMatchObject({ value: { time: 2n } })
  })

  it('fails and closes on a frame that does not authenticate', async () => {
    const { socket, relay } = opened()
    relay.startEncryption(FrameCipher.v0(nacl.randomBytes(32)))
    socket.receive(FrameCipher.v0(nacl.randomBytes(32)).seal(new Uint8Array(4)))
    await expect(relay.messages[Symbol.asyncIterator]().next()).rejects.toThrow(TransportError)
    expect(socket.closedWith).toBe(1000)
    expect(() => {
      relay.send(delay(1n))
    }).toThrow(TransportError)
  })

  it('fails and closes on a frame larger than the peer itself accepts', async () => {
    const { socket, relay } = opened()
    socket.receiveData(new ArrayBuffer(MAX_FRAME_BYTES + 1))
    const messages = relay.messages[Symbol.asyncIterator]()
    await expect(messages.next()).rejects.toThrow(
      `relay frame of ${MAX_FRAME_BYTES + 1} bytes exceeds ${MAX_FRAME_BYTES}`,
    )
    expect(socket.closedWith).toBe(1000)
  })

  it('fails once the unread backlog exceeds its frame bound, after what was already queued', async () => {
    const { socket, relay } = opened()
    const frame = toBinary(MessageSchema, delay(1n))
    for (let i = 0; i <= MAX_QUEUED_FRAMES; i++) socket.receive(frame)
    const messages = relay.messages[Symbol.asyncIterator]()
    for (let i = 0; i < MAX_QUEUED_FRAMES; i++) await pull(messages)
    await expect(messages.next()).rejects.toThrow(
      `relay backlog of ${MAX_QUEUED_FRAMES + 1} frames`,
    )
    expect(socket.closedWith).toBe(1000)
  })

  it('fails once the unread backlog exceeds its byte bound', async () => {
    const { socket, relay } = opened()
    socket.receive(toBinary(MessageSchema, delay(1n)))
    socket.receiveData(new ArrayBuffer(MAX_QUEUED_BYTES))
    const messages = relay.messages[Symbol.asyncIterator]()
    expect((await pull(messages)).union.case).toBe('testDelay')
    await expect(messages.next()).rejects.toThrow(
      `exceeds ${MAX_QUEUED_FRAMES} / ${MAX_QUEUED_BYTES}`,
    )
  })

  it('does not count frames already pulled against the backlog', async () => {
    const { socket, relay } = opened()
    const frame = toBinary(MessageSchema, delay(1n))
    const messages = relay.messages[Symbol.asyncIterator]()
    for (let i = 0; i < MAX_QUEUED_FRAMES * 2; i++) {
      socket.receive(frame)
      await pull(messages)
    }
    expect(socket.closedWith).toBeNull()
  })

  it('fails on a text frame', async () => {
    const { socket, relay } = opened()
    socket.receiveData('hello')
    await expect(relay.messages[Symbol.asyncIterator]().next()).rejects.toThrow(
      'relay sent a non-binary message',
    )
  })

  it('fails with the close code when the relay drops the connection', async () => {
    const { socket, relay } = opened()
    socket.receive(toBinary(MessageSchema, msg({ case: 'misc', value: create(MiscSchema) })))
    socket.drop(1006)
    const messages = relay.messages[Symbol.asyncIterator]()
    expect((await pull(messages)).union.case).toBe('misc')
    await expect(messages.next()).rejects.toThrow('relay connection closed (code 1006)')
  })

  it('ends quietly after a local close and ignores later frames', async () => {
    const { socket, relay } = opened()
    relay.close()
    socket.receive(toBinary(MessageSchema, delay(1n)))
    expect(await relay.messages[Symbol.asyncIterator]().next()).toEqual({
      done: true,
      value: undefined,
    })
    expect(socket.closedWith).toBe(1000)
  })

  it('refuses to send before the socket is open', () => {
    const socket = new FakeSocket('wss://console.test/ws/relay')
    const relay = relayWith(socket)
    expect(() => {
      relay.send(delay(1n))
    }).toThrow('relay connection is not open yet')
  })
})
