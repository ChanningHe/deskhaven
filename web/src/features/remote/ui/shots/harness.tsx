// Screenshot harness: the console as main.tsx starts it, with the remote page's server and
// device replaced by a FakeSession that a Playwright script drives through
// `window.__remoteShots`. Served by the Vite dev server only; the app never imports it.
import '@/index.css'
import '@/i18n'

import { RouterProvider } from '@tanstack/react-router'
import { createRoot } from 'react-dom/client'

import { ApiError, isErrorCode } from '@/api/api-error'
import { createApp } from '@/app/create-app'
import { AppProviders } from '@/app/providers'

import type { ConnectDeps } from '../../connect/controller'
import { FakeSession, OPENED } from '../../connect/fake-session'
import { RemoteDepsContext } from '../deps-context'

interface Failure {
  status: number
  code: string
  details?: Record<string, unknown>
}

interface Shots {
  session: FakeSession | null
  /** POST sessions fails with this while set. */
  failWith: Failure | null
  /** Logs the session in and shows a synthetic 1280×800 frame with the peer's cursor. */
  activate(displays: number): void
  navigate(href: string): Promise<void>
}

declare global {
  interface Window {
    __remoteShots?: Shots
  }
}

const WIDTH = 1280
const HEIGHT = 800

function fixtureFrame(): VideoFrame {
  const canvas = new OffscreenCanvas(WIDTH, HEIGHT)
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('no 2d context')
  const sky = ctx.createLinearGradient(0, 0, WIDTH, HEIGHT)
  sky.addColorStop(0, '#1e3a5f')
  sky.addColorStop(1, '#4a7ab0')
  ctx.fillStyle = sky
  ctx.fillRect(0, 0, WIDTH, HEIGHT)
  ctx.fillStyle = '#f4f4f5'
  ctx.fillRect(160, 120, 720, 440)
  ctx.fillStyle = '#d4d4d8'
  ctx.fillRect(160, 120, 720, 36)
  ctx.fillStyle = '#18181b'
  ctx.font = '28px sans-serif'
  ctx.fillText('Fixture frame 1280 × 800', 200, 220)
  ctx.fillStyle = '#27272a'
  ctx.fillRect(0, HEIGHT - 48, WIDTH, 48)
  return new VideoFrame(canvas, { timestamp: 0 })
}

/** A 12×18 arrow: white with a black outline, hotspot at its tip. */
function arrow(): Uint8ClampedArray<ArrayBuffer> {
  const w = 12
  const h = 18
  const rgba = new Uint8ClampedArray(w * h * 4)
  for (let y = 0; y < h; y++) {
    const span = Math.min(y, w - 1)
    for (let x = 0; x <= span; x++) {
      const edge = x === 0 || x === span || y === h - 1
      rgba.set(edge ? [0, 0, 0, 255] : [255, 255, 255, 255], (y * w + x) * 4)
    }
  }
  return rgba
}

const { queryClient, router } = createApp()

const shots: Shots = {
  session: null,
  failWith: null,
  activate(displays) {
    const session = shots.session
    if (!session) throw new Error('no session yet')
    session.activate(displays, 'Windows')
    session.cursor.set({
      shape: { id: '1', width: 12, height: 18, hotX: 0, hotY: 0, rgba: arrow() },
      position: { x: 900, y: 420 },
    })
    session.queue.push(fixtureFrame())
  },
  navigate: (href) => router.navigate({ href }),
}
window.__remoteShots = shots

const deps: ConnectDeps = {
  openSession: () => {
    const failure = shots.failWith
    if (!failure) return Promise.resolve(OPENED)
    const code = isErrorCode(failure.code) ? failure.code : undefined
    return Promise.reject(new ApiError(failure.status, { code, details: failure.details }))
  },
  closeSession: () => Promise.resolve(),
  probeDecoders: () => Promise.resolve({ vp9: true, av1: true, vp8: true }),
  createRelay: () => ({
    send: () => undefined,
    messages: { [Symbol.asyncIterator]: () => ({ next: () => new Promise(() => undefined) }) },
    close: () => undefined,
    startEncryption: () => undefined,
  }),
  startSession: () => {
    shots.session = new FakeSession()
    return shots.session
  },
  location: window.location,
}

const rootElement = document.getElementById('root')
if (!rootElement) throw new Error('missing #root element in index.html')

createRoot(rootElement).render(
  <AppProviders queryClient={queryClient}>
    <RemoteDepsContext.Provider value={deps}>
      <RouterProvider router={router} />
    </RemoteDepsContext.Provider>
  </AppProviders>,
)
