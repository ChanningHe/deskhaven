// Registers a device the way a RustDesk client does on its first start: UDP RegisterPeer,
// then RegisterPk (hbb_common rendezvous.proto), which creates the `devices` row. The
// protobuf messages are encoded by hand; only the fields used here exist.
import { createSocket } from 'node:dgram'
import type { Socket } from 'node:dgram'

const REPLY_TIMEOUT_MS = 5_000
/** An Ed25519 public key's length; its content does not matter to the server. */
const PK_BYTES = 32
const WIRE_VARINT = 0
const WIRE_LEN = 2
/** RendezvousMessage oneof fields. */
const REGISTER_PEER = 6
const REGISTER_PEER_RESPONSE = 7
const REGISTER_PK = 15
const REGISTER_PK_RESPONSE = 16
/** RegisterPkResponse.Result.OK */
const RESULT_OK = 0

function varint(value: number): Buffer {
  const bytes: number[] = []
  let rest = value
  while (rest > 0x7f) {
    bytes.push((rest & 0x7f) | 0x80)
    rest = Math.floor(rest / 128)
  }
  bytes.push(rest)
  return Buffer.from(bytes)
}

function lengthDelimited(field: number, payload: Buffer): Buffer {
  return Buffer.concat([varint((field << 3) | WIRE_LEN), varint(payload.length), payload])
}

interface Field {
  field: number
  /** Varint value, or the payload of a length-delimited field. */
  value: number | Buffer
}

function readVarint(bytes: Buffer, start: number): [number, number] {
  let value = 0
  let factor = 1
  let at = start
  for (;;) {
    const byte = bytes[at]
    if (byte === undefined) throw new Error('truncated varint')
    value += (byte & 0x7f) * factor
    at += 1
    if (byte < 0x80) return [value, at]
    factor *= 128
  }
}

/** The top-level fields of a message (varint and length-delimited wire types only). */
function fields(bytes: Buffer): Field[] {
  const out: Field[] = []
  let at = 0
  while (at < bytes.length) {
    const [key, afterKey] = readVarint(bytes, at)
    const [number, wire] = [key >>> 3, key & 0x7]
    if (wire === WIRE_VARINT) {
      const [value, next] = readVarint(bytes, afterKey)
      out.push({ field: number, value })
      at = next
    } else if (wire === WIRE_LEN) {
      const [length, start] = readVarint(bytes, afterKey)
      out.push({ field: number, value: bytes.subarray(start, start + length) })
      at = start + length
    } else {
      throw new Error(`unexpected wire type ${String(wire)}`)
    }
  }
  return out
}

/** The payload of the RendezvousMessage variant `field` in `reply`. */
function variant(reply: Buffer, field: number): Buffer {
  const found = fields(reply).find((f) => f.field === field)
  if (found === undefined || typeof found.value === 'number') {
    throw new Error(`expected RendezvousMessage field ${String(field)}`)
  }
  return found.value
}

/** Sends `message` from `socket` and resolves with the next datagram it receives. */
function exchange(socket: Socket, port: number, message: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.removeAllListeners('message')
      reject(new Error(`no UDP reply from the signal server on ${String(port)}`))
    }, REPLY_TIMEOUT_MS)
    socket.once('message', (reply) => {
      clearTimeout(timer)
      resolve(reply)
    })
    socket.send(message, port, '127.0.0.1')
  })
}

export interface Device {
  rustdeskId: string
  /** The machine uuid bytes; the HTTP API carries them base64-encoded. */
  uuid: Buffer
}

/** Registers `device` with the signal server on `port`; throws unless the result is OK. */
export async function registerDevice(port: number, device: Device): Promise<void> {
  const id = Buffer.from(device.rustdeskId)
  const pk = Buffer.concat([
    lengthDelimited(1, id),
    lengthDelimited(2, device.uuid),
    lengthDelimited(3, Buffer.alloc(PK_BYTES, 7)),
  ])
  const socket = createSocket('udp4')
  try {
    const peer = await exchange(
      socket,
      port,
      lengthDelimited(REGISTER_PEER, lengthDelimited(1, id)),
    )
    variant(peer, REGISTER_PEER_RESPONSE)
    const reply = await exchange(socket, port, lengthDelimited(REGISTER_PK, pk))
    const result = fields(variant(reply, REGISTER_PK_RESPONSE)).find((f) => f.field === 1)
    const code = result?.value ?? RESULT_OK
    if (code !== RESULT_OK) throw new Error(`RegisterPk refused: result ${String(code)}`)
  } finally {
    socket.close()
  }
}
