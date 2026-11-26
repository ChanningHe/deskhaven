import { act, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { SAMPLE_VERSION, type VideoSample } from '../video/sample-export'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
})

async function activePage() {
  const f = fakeDeps()
  const view = renderWithProviders(
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId="123456789" />
    </RemoteDepsContext.Provider>,
  )
  await waitFor(() => {
    expect(f.sessions).toHaveLength(1)
  })
  const [session] = f.sessions
  if (!session) throw new Error('no session started')
  act(() => {
    session.activate()
  })
  return { ...view, session, f }
}

async function showStats(user: Awaited<ReturnType<typeof activePage>>['user']) {
  await openMenu(user, '显示')
  await user.click(screen.getByRole('menuitemcheckbox', { name: '统计信息' }))
}

const SAMPLE: VideoSample = {
  version: SAMPLE_VERSION,
  clientVersion: '1.5.0',
  codec: 'vp9s',
  complete: true,
  peer: { platform: 'Linux', version: '1.5.0' },
  display: { width: 1280, height: 800 },
  frames: [{ key: true, pts: 16, data: 'AAAAAQ==' }],
}

describe('video sample export', () => {
  it('saves the recorded sample as a JSON file named after the codec', async () => {
    const urls: Blob[] = []
    const revoke = vi.fn()
    Object.assign(URL, {
      createObjectURL: (blob: Blob) => {
        urls.push(blob)
        return 'blob:sample'
      },
      revokeObjectURL: revoke,
    })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      const { user, session } = await activePage()
      await showStats(user)
      await user.click(screen.getByRole('button', { name: '导出视频样本' }))
      act(() => {
        session.finishSample(SAMPLE)
      })
      await waitFor(() => {
        expect(click).toHaveBeenCalledTimes(1)
      })
      const anchor = click.mock.contexts[0] as HTMLAnchorElement
      expect(anchor.download).toMatch(/^video-sample-vp9s-\d{8}-\d{6}\.json$/)
      expect(anchor.href).toBe('blob:sample')
      const [blob] = urls
      if (!blob) throw new Error('no blob')
      expect(JSON.parse(await blob.text())).toEqual(SAMPLE)
      expect(revoke).not.toHaveBeenCalled()
    } finally {
      click.mockRestore()
    }
  })
})
