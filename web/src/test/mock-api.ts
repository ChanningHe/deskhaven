import { vi } from 'vitest'

export interface MockResponse {
  status?: number
  body?: unknown
  headers?: Record<string, string>
}

export type MockHandler = (request: {
  url: URL
  init: RequestInit
}) => MockResponse | Promise<MockResponse>

export function toResponse({ status = 200, body, headers = {} }: MockResponse): Response {
  if (body === undefined) return new Response(null, { status, headers })
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  })
}

function urlOf(input: RequestInfo | URL): URL {
  const href = input instanceof Request ? input.url : input instanceof URL ? input.href : input
  return new URL(href, 'http://localhost')
}

export function errorEnvelope(code: string, message = code) {
  return { error: { code, message } }
}

/** Keyed by `"METHOD /path"` (query ignored); unmatched requests answer 404 `not_found`. */
export function mockApi(routes: Record<string, MockResponse | MockHandler>) {
  const spy = vi.fn((input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = urlOf(input)
    const method = (init.method ?? 'GET').toUpperCase()
    const route = routes[`${method} ${url.pathname}`]
    if (route === undefined) {
      return Promise.resolve(toResponse({ status: 404, body: errorEnvelope('not_found') }))
    }
    const response = typeof route === 'function' ? route({ url, init }) : route
    return Promise.resolve(response).then(toResponse)
  })
  vi.stubGlobal('fetch', spy)
  return spy
}

export function callsTo(spy: ReturnType<typeof mockApi>, pathname: string): number {
  return spy.mock.calls.filter(([input]) => urlOf(input).pathname === pathname).length
}

export function requestedUrl(spy: ReturnType<typeof mockApi>, index: number): URL {
  const call = spy.mock.calls[index]
  if (!call) throw new Error(`no fetch call #${index}`)
  return urlOf(call[0])
}
