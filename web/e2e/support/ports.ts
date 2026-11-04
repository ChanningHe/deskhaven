// A signal port block for the server under test. The embedded hbbs listens on TCP
// port-1 (NAT test), port, port+2 (WebSocket) and UDP port, so it cannot take port 0.
import { createSocket } from 'node:dgram'
import { createServer } from 'node:net'

/** First candidate block (above registered-service ports, below the Linux ephemeral range). */
const FIRST_PORT = 23_000
const BLOCK = 10
const BLOCKS = 500

function tcpFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = createServer()
    server.once('error', () => {
      resolve(false)
    })
    server.listen({ port, host: '0.0.0.0', exclusive: true }, () => {
      server.close(() => {
        resolve(true)
      })
    })
  })
}

function udpFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createSocket('udp4')
    socket.once('error', () => {
      socket.close()
      resolve(false)
    })
    socket.bind(port, '0.0.0.0', () => {
      socket.close(() => {
        resolve(true)
      })
    })
  })
}

async function blockFree(port: number): Promise<boolean> {
  const tcp = await Promise.all([port - 1, port, port + 2].map(tcpFree))
  return tcp.every(Boolean) && (await udpFree(port))
}

/** A signal port whose block was free a moment ago (the suite runs one server at a time). */
export async function freeSignalPort(): Promise<number> {
  const offset = Math.floor(Math.random() * BLOCKS)
  for (let i = 0; i < BLOCKS; i += 1) {
    const port = FIRST_PORT + 1 + ((offset + i) % BLOCKS) * BLOCK
    if (await blockFree(port)) return port
  }
  throw new Error('no free signal port block')
}
