// The smallest reverse proxy shaped like deploy/compose/Caddyfile: `/ws/relay*` goes to the
// relay's WebSocket port, everything else to the console. The browser then sees one origin
// and the console's `connect-src 'self'` holds, as behind a real proxy.
import { createServer, request } from 'node:http'
import type { IncomingMessage, Server } from 'node:http'
import { connect } from 'node:net'
import type { Socket } from 'node:net'

const RELAY_PATH = '/ws/relay'

export interface Upstream {
  host: string
  port: number
}

export interface Shaping {
  rttMs?: number
  kbps?: number
}

// Past BUFFER_BYTES the proxy stops reading, so TCP pushes back to the sender.
export const BUFFER_BYTES = 64 * 1024

export interface Proxy {
  /** `http://127.0.0.1:<port>`. */
  origin: string
  dropRelay: () => void
  refuseRelay: (on: boolean) => void
  stop: () => Promise<void>
}

/** The raw head of an upgrade request, replayed to the upstream. */
function upgradeHead(req: IncomingMessage): string {
  const lines = [`${req.method ?? 'GET'} ${req.url ?? '/'} HTTP/1.1`]
  for (let i = 0; i < req.rawHeaders.length; i += 2) {
    lines.push(`${req.rawHeaders[i] ?? ''}: ${req.rawHeaders[i + 1] ?? ''}`)
  }
  return `${lines.join('\r\n')}\r\n\r\n`
}

function shapedPipe(from: Socket, to: Socket, delayMs: number, kbps: number | undefined) {
  const queue: { due: number; chunk: Buffer }[] = []
  let linkFreeAt = 0
  let timer: NodeJS.Timeout | undefined
  const fullUntil = () => (kbps === undefined ? 0 : linkFreeAt - (BUFFER_BYTES * 8) / kbps)
  const backlogFull = () => fullUntil() > performance.now()
  const gate = () => {
    if (backlogFull() || to.writableNeedDrain) from.pause()
    else from.resume()
  }
  const pump = () => {
    timer = undefined
    if (to.destroyed) return
    const now = performance.now()
    while (queue.length > 0 && (queue[0]?.due ?? Infinity) <= now) {
      const next = queue.shift()
      if (next) to.write(next.chunk)
    }
    gate()
    const head = queue[0]
    const wake = Math.min(head?.due ?? Infinity, backlogFull() ? fullUntil() : Infinity)
    if (wake !== Infinity) timer = setTimeout(pump, Math.max(0, wake - now))
  }
  from.on('data', (chunk: Buffer) => {
    const now = performance.now()
    const start = Math.max(now, linkFreeAt)
    linkFreeAt = kbps === undefined ? start : start + (chunk.length * 8) / kbps
    queue.push({ due: linkFreeAt + delayMs, chunk })
    if (timer === undefined) pump()
    else gate()
  })
  to.on('drain', gate)
  to.on('close', () => {
    clearTimeout(timer)
    queue.length = 0
  })
}

function shapes(shaping: Shaping): boolean {
  return (shaping.rttMs ?? 0) > 0 || shaping.kbps !== undefined
}

export async function startProxy(
  consoleUp: Upstream,
  relayWs: Upstream,
  shaping: Shaping = {},
): Promise<Proxy> {
  if (shaping.kbps !== undefined && !(shaping.kbps > 0)) throw new Error('kbps must be > 0')
  const sockets = new Set<Socket>()
  const relays = new Set<() => void>()
  let refusing = false
  const server: Server = createServer((req, res) => {
    const upstream = request(
      {
        host: consoleUp.host,
        port: consoleUp.port,
        method: req.method,
        path: req.url,
        headers: req.headers,
      },
      (reply) => {
        res.writeHead(reply.statusCode ?? 502, reply.headers)
        reply.pipe(res)
      },
    )
    upstream.on('error', () => {
      if (!res.headersSent) res.writeHead(502)
      res.end()
    })
    req.pipe(upstream)
  })
  server.on('upgrade', (req: IncomingMessage, client: Socket, head: Buffer) => {
    const relay = req.url?.startsWith(RELAY_PATH) === true
    if (relay && refusing) {
      client.destroy()
      return
    }
    const target = relay ? relayWs : consoleUp
    const upstream = connect(target.port, target.host, () => {
      upstream.write(upgradeHead(req))
      if (head.length > 0) upstream.write(head)
      if (relay && shapes(shaping)) {
        const oneWay = (shaping.rttMs ?? 0) / 2
        shapedPipe(upstream, client, oneWay, shaping.kbps)
        shapedPipe(client, upstream, oneWay, shaping.kbps)
        return
      }
      upstream.pipe(client)
      client.pipe(upstream)
    })
    sockets.add(client)
    sockets.add(upstream)
    const drop = () => {
      client.destroy()
      upstream.destroy()
      relays.delete(drop)
    }
    if (relay) relays.add(drop)
    client.on('error', drop)
    upstream.on('error', drop)
    client.on('close', drop)
    upstream.on('close', drop)
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  if (address === null || typeof address === 'string') throw new Error('proxy has no port')
  return {
    origin: `http://127.0.0.1:${String(address.port)}`,
    dropRelay: () => {
      for (const drop of [...relays]) drop()
    },
    refuseRelay: (on) => {
      refusing = on
    },
    stop: () =>
      new Promise<void>((resolve) => {
        for (const socket of sockets) socket.destroy()
        server.closeAllConnections()
        server.close(() => {
          resolve()
        })
      }),
  }
}
