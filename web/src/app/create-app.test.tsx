import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { HttpError } from '@/lib/http-error'
import { renderApp } from '@/test/render-app'

describe('createApp', () => {
  it('stays on the page for other API errors', async () => {
    const { queryClient, router } = renderApp('/devices')
    await screen.findByRole('heading', { level: 1, name: '设备' })

    await expect(
      queryClient.query({
        queryKey: ['devices'],
        queryFn: () => Promise.reject(new HttpError(403)),
      }),
    ).rejects.toThrow()

    expect(router.state.location.pathname).toBe('/devices')
  })
})
