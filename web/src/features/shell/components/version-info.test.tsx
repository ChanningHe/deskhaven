import type { QueryClient } from '@tanstack/react-query'
import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo } from '@/test/mock-api'
import type { MockResponse } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

const VERSION_QUERY_KEY = ['server', 'version']
const VERSION_ROUTE = 'GET /api/version'
const VERSION = { version: '0.1.0-rc.4', source_url: 'https://github.com/ChanningHe/deskhaven' }

function versionApi(response: MockResponse = { body: VERSION }) {
  return { [VERSION_ROUTE]: response }
}

async function renderShell(response?: MockResponse) {
  const app = renderApp('/', { api: versionApi(response) })
  await screen.findByRole('navigation', { name: '主导航' })
  return app
}

/** Waits until the version request has finished, so an absence check is not premature. */
async function querySettled(queryClient: QueryClient) {
  await waitFor(() => {
    expect(queryClient.getQueryState(VERSION_QUERY_KEY)?.fetchStatus).toBe('idle')
    expect(queryClient.getQueryState(VERSION_QUERY_KEY)?.status).not.toBe('pending')
  })
}

function sidebar(): HTMLElement {
  const element = document.getElementById('app-sidebar')
  if (element === null) throw new Error('no sidebar')
  return element
}

describe('server version', () => {
  it('links the version to a valid https source', async () => {
    const { fetchSpy } = await renderShell()

    expect(await within(sidebar()).findByText('v0.1.0-rc.4')).toBeInTheDocument()
    const link = within(sidebar()).getByRole('link', { name: 'v0.1.0-rc.4' })
    expect(link).toHaveAttribute('href', 'https://github.com/ChanningHe/deskhaven')
    expect(link).toHaveAttribute('target', '_blank')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(callsTo(fetchSpy, '/api/version')).toBe(1)
  })

  it.each([
    ['a non-https source_url', { version: '0.1.0', source_url: 'http://example.com/src' }],
    ['a javascript: source_url', { version: '0.1.0', source_url: 'javascript:alert(1)' }],
    ['a non-string version', { version: 1, source_url: 'https://example.com/src' }],
    ['a missing source_url', { version: '0.1.0' }],
  ])('renders nothing for %s', async (_name, body) => {
    const { queryClient } = await renderShell({ body })

    await querySettled(queryClient)
    expect(screen.queryByText(/^v\d/)).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: /^v\d/ })).not.toBeInTheDocument()
  })
})
