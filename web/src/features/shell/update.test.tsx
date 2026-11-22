import type { QueryClient } from '@tanstack/react-query'
import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { Me } from '@/api/generated/model'
import { ADMIN_ME, USER_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import type { MockResponse } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { SIDEBAR_COLLAPSED_STORAGE_KEY } from './use-sidebar-collapsed'

const UPDATE_PATH = '/admin/api/v1/update-check'
const UPDATE_QUERY_KEY = [UPDATE_PATH]
const RELEASE_URL = 'https://github.com/ChanningHe/deskhaven/releases/tag/v99.0.0'
const UPDATE = { latest: '99.0.0', release_url: RELEASE_URL }
const UPDATE_NAME = '有新版本 v99.0.0'
const VERSION = { version: '0.1.1', source_url: 'https://github.com/ChanningHe/deskhaven' }
/** The version line exactly as it rendered before update checks existed. */
const PLAIN_VERSION_LINE =
  '<p class="flex text-xs text-muted-foreground min-w-0"><a href="https://github.com/ChanningHe/deskhaven" target="_blank" rel="noopener noreferrer" title="查看源代码" class="truncate underline-offset-4 hover:text-foreground hover:underline">v0.1.1</a></p>'

function api(update: MockResponse = { body: UPDATE }) {
  return {
    'GET /api/version': { body: VERSION },
    [`GET ${UPDATE_PATH}`]: update,
  }
}

async function renderShell(update?: MockResponse, me: Me = ADMIN_ME, path = '/') {
  const app = renderApp(path, { me, api: api(update) })
  await screen.findByRole('navigation', { name: '主导航' })
  return app
}

function sidebar(): HTMLElement {
  const element = document.getElementById('app-sidebar')
  if (element === null) throw new Error('no sidebar')
  return element
}

async function updateSettled(queryClient: QueryClient) {
  await waitFor(() => {
    const state = queryClient.getQueryState(UPDATE_QUERY_KEY)
    expect(state?.fetchStatus).toBe('idle')
    expect(state?.status).not.toBe('pending')
  })
}

describe('update indicator', () => {
  it('links to the release page next to the version for an admin', async () => {
    await renderShell()

    const link = await within(sidebar()).findByRole('link', { name: UPDATE_NAME })
    expect(link).toHaveAttribute('href', RELEASE_URL)
    expect(link).toHaveAttribute('target', '_blank')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(within(sidebar()).getByRole('link', { name: 'v0.1.1' })).toBeInTheDocument()
  })

  it('shows the version as a tooltip on hover', async () => {
    const { user } = await renderShell()

    await user.hover(await within(sidebar()).findByRole('link', { name: UPDATE_NAME }))

    expect(
      await screen.findByText(UPDATE_NAME, { selector: '[data-slot=tooltip-content]' }),
    ).toBeInTheDocument()
  })

  it('leaves the version line unchanged when there is no update', async () => {
    const { queryClient } = await renderShell({ body: { latest: null, release_url: null } })

    const version = await within(sidebar()).findByRole('link', { name: 'v0.1.1' })
    await updateSettled(queryClient)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
    expect(version.parentElement?.outerHTML).toBe(PLAIN_VERSION_LINE)
  })

  it('shows nothing, and does not retry, when the check fails', async () => {
    const { queryClient, fetchSpy } = await renderShell({
      status: 500,
      body: errorEnvelope('internal'),
    })

    const version = await within(sidebar()).findByRole('link', { name: 'v0.1.1' })
    await updateSettled(queryClient)
    expect(callsTo(fetchSpy, UPDATE_PATH)).toBe(1)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
    expect(within(sidebar()).queryByRole('alert')).not.toBeInTheDocument()
    expect(version.parentElement?.outerHTML).toBe(PLAIN_VERSION_LINE)
  })

  it.each([
    ['a javascript: release_url', { latest: '99.0.0', release_url: 'javascript:alert(1)' }],
    ['an http release_url', { latest: '99.0.0', release_url: 'http://github.com/x' }],
    ['an unparsable release_url', { latest: '99.0.0', release_url: 'not a url' }],
    ['an empty latest', { latest: '', release_url: RELEASE_URL }],
    ['a missing latest', { release_url: RELEASE_URL }],
  ])('shows nothing for %s', async (_name, body) => {
    const { queryClient } = await renderShell({ body })

    await within(sidebar()).findByRole('link', { name: 'v0.1.1' })
    await updateSettled(queryClient)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
  })

  it('never asks for a user without admin rights', async () => {
    const { fetchSpy } = await renderShell(undefined, USER_ME, '/me')

    await within(sidebar()).findByRole('link', { name: 'v0.1.1' })
    expect(callsTo(fetchSpy, UPDATE_PATH)).toBe(0)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
  })

  it('is absent from the collapsed sidebar', async () => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, 'true')
    const { queryClient } = await renderShell()

    await updateSettled(queryClient)
    expect(within(sidebar()).queryByRole('link', { name: UPDATE_NAME })).not.toBeInTheDocument()
  })

  it('shows in the mobile drawer', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '打开导航' }))
    const drawer = await screen.findByRole('dialog')

    expect(await within(drawer).findByRole('link', { name: UPDATE_NAME })).toHaveAttribute(
      'href',
      RELEASE_URL,
    )
  })

  it('is absent from the sign-in page, which never asks', async () => {
    const { fetchSpy } = renderApp('/login', { me: null, api: api() })

    expect(await screen.findByRole('link', { name: 'v0.1.1' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, UPDATE_PATH)).toBe(0)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
  })
})
