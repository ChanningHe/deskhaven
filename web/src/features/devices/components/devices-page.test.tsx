import { waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import { pageOf } from '@/features/users/test-data'

import { DEVICE_GROUPS_PATH, DEVICES_PATH } from '../test-data'

function listCalls(fetchSpy: Parameters<typeof requestedUrl>[0]) {
  return fetchSpy.mock.calls
    .map((_, index) => requestedUrl(fetchSpy, index))
    .filter((url) => url.pathname === DEVICES_PATH)
}

describe('devices page', () => {
  it('reads the filters from the URL', async () => {
    const { fetchSpy } = renderApp('/devices?online=false&status=disabled&sort=-last_seen_at', {
      api: {
        [`GET ${DEVICES_PATH}`]: pageOf([]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([]),
      },
    })
    await waitFor(() => {
      expect(listCalls(fetchSpy)).not.toHaveLength(0)
    })
    const params = listCalls(fetchSpy)[0]?.searchParams
    expect(params?.get('online')).toBe('false')
    expect(params?.get('status')).toBe('disabled')
    expect(params?.get('sort')).toBe('-last_seen_at')
  })
})
