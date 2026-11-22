import { decodeBase64 } from '../bytes'
import session145 from './1.4.5/session.json'
import session150 from './1.5.0/session.json'

export type FixtureVersion = '1.5.0' | '1.4.5'

/** Captured wire frames, one per file, stored as base64 of the raw bytes. */
export type FixtureFile =
  | 'hbbs-relay-response'
  | 'hbbr-signed-id'
  | 'hbbr-public-key'
  | 'hbbr-enc-01-Hash'
  | 'hbbr-enc-06-VideoFrame'

const frames = import.meta.glob<string>('./*/*.b64', {
  query: '?raw',
  import: 'default',
  eager: true,
})

export function fixtureBytes(version: FixtureVersion, file: FixtureFile): Uint8Array {
  const text = frames[`./${version}/${file}.b64`]
  if (text === undefined) throw new Error(`missing fixture ${version}/${file}`)
  return decodeBase64(text.trim())
}

export interface FixtureSession {
  peerId: string
  serverPub: string
  devicePk: Uint8Array
  peerBoxPk: Uint8Array
  peerKxVersion: number
  controllerBoxPk: Uint8Array
  controllerBoxSk: Uint8Array
  symmetricKey: Uint8Array
  password: string
  salt: string
  challenge: string
}

export function fixtureSession(version: FixtureVersion): FixtureSession {
  const s = version === '1.5.0' ? session150 : session145
  return {
    peerId: s.peer_id,
    serverPub: s.server_pub,
    devicePk: decodeBase64(s.device_pk),
    peerBoxPk: decodeBase64(s.peer_box_pk),
    peerKxVersion: s.peer_kx_version,
    controllerBoxPk: decodeBase64(s.controller_box_pk),
    controllerBoxSk: decodeBase64(s.controller_box_sk),
    symmetricKey: decodeBase64(s.symmetric_key),
    password: s.password,
    salt: s.salt,
    challenge: s.challenge,
  }
}

export const FIXTURE_VERSIONS: readonly FixtureVersion[] = ['1.5.0', '1.4.5']
