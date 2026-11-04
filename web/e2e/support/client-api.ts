// The RustDesk client's HTTP API, called the way the client calls it (JSON bodies,
// Bearer tokens), for the client-side half of the flows.
import type { Device } from './rendezvous.ts'

export interface ClientReply {
  status: number
  body: Record<string, unknown>
}

async function post(
  baseUrl: string,
  path: string,
  body: unknown,
  token?: string,
): Promise<ClientReply> {
  const response = await fetch(new URL(path, baseUrl), {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(token === undefined ? {} : { Authorization: `Bearer ${token}` }),
    },
    body: JSON.stringify(body),
  })
  const text = await response.text()
  const parsed: unknown = text === '' ? {} : JSON.parse(text)
  return { status: response.status, body: parsed as Record<string, unknown> }
}

/** `POST /api/login` with a password from `device`; the access token on success. */
export async function clientLogin(
  baseUrl: string,
  device: Device,
  username: string,
  password: string,
): Promise<string> {
  const reply = await post(baseUrl, '/api/login', {
    type: 'account',
    username,
    password,
    id: device.rustdeskId,
    uuid: device.uuid.toString('base64'),
    deviceInfo: { os: 'linux', type: 'client', name: 'e2e' },
  })
  const token = reply.body.access_token
  if (reply.status !== 200 || typeof token !== 'string') {
    throw new Error(`client login failed: ${String(reply.status)} ${JSON.stringify(reply.body)}`)
  }
  return token
}

/** `POST /api/currentUser` with `token`. */
export function currentUser(baseUrl: string, device: Device, token: string): Promise<ClientReply> {
  const body = { id: device.rustdeskId, uuid: device.uuid.toString('base64') }
  return post(baseUrl, '/api/currentUser', body, token)
}

/** One `POST /api/heartbeat` of an idle client that has no strategy yet. */
export function heartbeat(baseUrl: string, device: Device): Promise<ClientReply> {
  return post(baseUrl, '/api/heartbeat', {
    id: device.rustdeskId,
    uuid: device.uuid.toString('base64'),
    ver: 1_004_002,
    modified_at: 0,
  })
}
