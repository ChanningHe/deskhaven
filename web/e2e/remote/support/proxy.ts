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

export interface Proxy {
  /** `http://127.0.0.1:<port>`. */
  origin: string
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

export async function startProxy(consoleUp: Upstream, relayWs: Upstream): Promise<Proxy> {
  const sockets = new Set<Socket>()
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
    const target = req.url?.startsWith(RELAY_PATH) ? relayWs : consoleUp
    const upstream = connect(target.port, target.host, () => {
      upstream.write(upgradeHead(req))
      if (head.length > 0) upstream.write(head)
      upstream.pipe(client)
      client.pipe(upstream)
    })
    sockets.add(client)
    sockets.add(upstream)
    const drop = () => {
      client.destroy()
      upstream.destroy()
    }
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
