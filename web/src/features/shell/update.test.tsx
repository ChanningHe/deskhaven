import type { QueryClient } from '@tanstack/react-query'
import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { Me } from '@/api/generated/model'
import { ADMIN_ME, USER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import type { MockResponse } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

const UPDATE_PATH = '/admin/api/v1/update-check'
const UPDATE_QUERY_KEY = [UPDATE_PATH]
const RELEASE_URL = 'https://github.com/ChanningHe/deskhaven/releases/tag/v99.0.0'
const UPDATE = { latest: '99.0.0', release_url: RELEASE_URL }
const UPDATE_NAME = '有新版本 v99.0.0'
const VERSION = { version: '0.1.1', source_url: 'https://github.com/ChanningHe/deskhaven' }
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

  it('is absent from the sign-in page, which never asks', async () => {
    const { fetchSpy } = renderApp('/login', { me: null, api: api() })

    expect(await screen.findByRole('link', { name: 'v0.1.1' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, UPDATE_PATH)).toBe(0)
    expect(screen.queryByRole('link', { name: /有新版本/ })).not.toBeInTheDocument()
  })
})
