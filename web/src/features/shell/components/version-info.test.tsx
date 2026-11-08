import type { QueryClient } from '@tanstack/react-query'
import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo, errorEnvelope } from '@/test/mock-api'
import type { MockResponse } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { SIDEBAR_COLLAPSED_STORAGE_KEY } from '../use-sidebar-collapsed'

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
  it('shows the version and an external source link in the sidebar', async () => {
    const { fetchSpy } = await renderShell()

    expect(await within(sidebar()).findByText('v0.1.0-rc.4')).toBeInTheDocument()
    const link = within(sidebar()).getByRole('link', { name: '源代码' })
    expect(link).toHaveAttribute('href', 'https://github.com/ChanningHe/deskhaven')
    expect(link).toHaveAttribute('target', '_blank')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(callsTo(fetchSpy, '/api/version')).toBe(1)
  })

  it('shows nothing when the request fails', async () => {
    const { queryClient, fetchSpy } = await renderShell({
      status: 500,
      body: errorEnvelope('internal'),
    })

    await querySettled(queryClient)
    expect(callsTo(fetchSpy, '/api/version')).toBe(1)
    expect(screen.queryByText(/^v\d/)).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: '源代码' })).not.toBeInTheDocument()
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
    expect(screen.queryByRole('link', { name: '源代码' })).not.toBeInTheDocument()
  })

  it('hides the text when the sidebar is collapsed', async () => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, 'true')
    const { fetchSpy } = await renderShell()

    await waitFor(() => {
      expect(callsTo(fetchSpy, '/api/version')).toBe(0)
    })
    expect(within(sidebar()).queryByText('v0.1.0-rc.4')).not.toBeInTheDocument()
    expect(within(sidebar()).getByRole('button', { name: '切换侧栏' })).toBeInTheDocument()
  })

  it('shows it again after expanding the collapsed sidebar', async () => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, 'true')
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '切换侧栏' }))

    expect(await within(sidebar()).findByText('v0.1.0-rc.4')).toBeInTheDocument()
  })

  it('shows it at the bottom of the mobile drawer', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '打开导航' }))
    const drawer = await screen.findByRole('dialog')

    expect(await within(drawer).findByText('v0.1.0-rc.4')).toBeInTheDocument()
    expect(within(drawer).getByRole('link', { name: '源代码' })).toHaveAttribute(
      'href',
      'https://github.com/ChanningHe/deskhaven',
    )
  })

  it('shows it under the sign-in card', async () => {
    renderApp('/login', { me: null, api: versionApi() })

    expect(await screen.findByText('v0.1.0-rc.4')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '源代码' })).toHaveAttribute(
      'href',
      'https://github.com/ChanningHe/deskhaven',
    )
  })
})
