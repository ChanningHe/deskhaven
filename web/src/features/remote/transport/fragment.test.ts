import { describe, expect, it } from 'vitest'

import {
  FRAG_END,
  FRAG_MORE,
  MAX_FRAGMENT_PAYLOAD,
  MAX_MESSAGE_BYTES,
  Reassembler,
  fragment,
} from './fragment'

function bytes(length: number): Uint8Array<ArrayBuffer> {
  return Uint8Array.from({ length }, (_, i) => i % 251)
}

function frame(header: number, payload: Uint8Array): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(payload.length + 1)
  out[0] = header
  out.set(payload, 1)
  return out
}

describe('fragment', () => {
  it('sends a short message as one FRAG_END fragment', () => {
    expect(fragment(bytes(3))).toEqual([frame(FRAG_END, bytes(3))])
  })

  it('sends an empty message as a lone FRAG_END header, never a zero-length message', () => {
    expect(fragment(new Uint8Array())).toEqual([Uint8Array.of(FRAG_END)])
  })

  it('splits at 60000-byte payloads, FRAG_MORE until the last', () => {
    const message = bytes(150_000)
    const parts = fragment(message)
    expect(parts.map((p) => [p[0], p.length])).toEqual([
      [FRAG_MORE, MAX_FRAGMENT_PAYLOAD + 1],
      [FRAG_MORE, MAX_FRAGMENT_PAYLOAD + 1],
      [FRAG_END, 30_001],
    ])
  })

  it('sends exactly one fragment at the payload limit', () => {
    expect(fragment(bytes(MAX_FRAGMENT_PAYLOAD)).map((p) => p[0])).toEqual([FRAG_END])
  })
})

describe('Reassembler', () => {
  it('reassembles what fragment split', () => {
    const message = bytes(150_000)
    const r = new Reassembler()
    const out = fragment(message).map((p) => r.push(p))
    expect(out.slice(0, 2)).toEqual([null, null])
    expect(out[2]).toEqual(message)
  })

  it('hands a whole message straight through, and an empty one as empty', () => {
    const r = new Reassembler()
    expect(r.push(frame(FRAG_END, bytes(5)))).toEqual(bytes(5))
    expect(r.push(Uint8Array.of(FRAG_END))).toEqual(new Uint8Array())
  })

  it('starts the next message clean after one completes', () => {
    const r = new Reassembler()
    r.push(frame(FRAG_MORE, bytes(2)))
    expect(r.push(frame(FRAG_END, bytes(1)))).toEqual(Uint8Array.of(0, 1, 0))
    expect(r.push(frame(FRAG_END, bytes(1)))).toEqual(Uint8Array.of(0))
  })

  it.each([
    ['an empty data-channel message', new Uint8Array()],
    ['a FRAG_MORE without payload', Uint8Array.of(FRAG_MORE)],
    ['an unknown header', Uint8Array.of(2, 0)],
  ])('rejects %s', (_, chunk) => {
    expect(() => new Reassembler().push(chunk)).toThrow()
  })

  it('rejects a message that would grow past the cap before buffering it', () => {
    const r = new Reassembler(10)
    expect(r.push(frame(FRAG_MORE, bytes(6)))).toBeNull()
    expect(() => r.push(frame(FRAG_END, bytes(5)))).toThrow(/exceeds 10/)
  })

  it('accepts a message of exactly the cap', () => {
    const r = new Reassembler(10)
    r.push(frame(FRAG_MORE, bytes(6)))
    expect(r.push(frame(FRAG_END, bytes(4)))).toHaveLength(10)
  })

  it('caps at the relay frame limit by default', () => {
    expect(MAX_MESSAGE_BYTES).toBe(64 << 20)
  })
})
