// The open drawer follows the server (polling) without losing the user's edits or writing back
// values the user never touched, and says so when it can no longer refresh.

import { act, screen, waitFor, within } from '@testing-library/react'
import type { QueryClient } from '@tanstack/react-query'
import { describe, expect, it } from 'vitest'

import { getGetDeviceQueryKey } from '@/api/generated/devices/devices'
import { errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { ALPHA, ALPHA_PATH, deviceDetail, drawerApi } from '../test-data'

const CAROL = { id: 6, username: 'carol' }
const OFFICE_REF = { id: 3, name: 'Office' }

/** The next poll, as a user of the page would trigger it by waiting. */
async function poll(queryClient: QueryClient) {
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: getGetDeviceQueryKey(1) })
  })
}

describe('device drawer: polling', () => {
  it('saves only the edited field when another admin changed other fields meanwhile', async () => {
    let served = deviceDetail(ALPHA)
    const bodies: unknown[] = []
    const { user, queryClient } = renderApp('/devices?device=1', {
      api: drawerApi(
        {
          [`PATCH ${ALPHA_PATH}`]: ({ init }) => {
            bodies.push(JSON.parse(init.body as string))
            served = deviceDetail({ ...served.device, note: 'rack 4' })
            return { body: served.device }
          },
        },
        () => ({ body: served }),
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const note = within(drawer).getByLabelText('备注')
    await user.clear(note)
    await user.type(note, 'rack 4')

    // Another administrator hands the device to carol and puts it in a group.
    served = deviceDetail({ ...ALPHA, owner: CAROL, device_group: OFFICE_REF })
    await poll(queryClient)

    expect(await within(drawer).findByText('carol')).toBeInTheDocument()
    expect(within(drawer).getByLabelText('设备组')).toHaveValue('3')
    expect(note).toHaveValue('rack 4')
    const save = within(drawer).getByRole('button', { name: '保存修改' })
    expect(save).toBeEnabled()

    await user.click(save)
    await waitFor(() => {
      expect(bodies).toEqual([{ note: 'rack 4' }])
    })
    expect(await within(drawer).findByText('已保存')).toBeInTheDocument()
    expect(save).toBeDisabled()
    expect(within(drawer).getByLabelText('备注')).toHaveValue('rack 4')
    expect(within(drawer).getByText('carol')).toBeInTheDocument()
  })

  it('keeps what the user typed when the server changed the same field, and warns', async () => {
    let served = deviceDetail(ALPHA)
    const bodies: unknown[] = []
    const { user, queryClient } = renderApp('/devices?device=1', {
      api: drawerApi(
        {
          [`PATCH ${ALPHA_PATH}`]: ({ init }) => {
            bodies.push(JSON.parse(init.body as string))
            return { body: ALPHA }
          },
        },
        () => ({ body: served }),
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const note = within(drawer).getByLabelText('备注')
    await user.clear(note)
    await user.type(note, 'mine')

    served = deviceDetail({ ...ALPHA, note: 'theirs' })
    await poll(queryClient)

    expect(await within(drawer).findByText(/他人刚修改了你正在编辑的内容/)).toBeInTheDocument()
    expect(note).toHaveValue('mine')
    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ note: 'mine' }])
    })
  })

  it('says the data is stale when a later poll fails, and keeps the last copy', async () => {
    let failing = false
    const { queryClient } = renderApp('/devices?device=1', {
      api: drawerApi({}, () =>
        failing
          ? { status: 429, body: errorEnvelope('rate_limited') }
          : { body: deviceDetail(ALPHA) },
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })

    failing = true
    await poll(queryClient)

    const alert = await within(drawer).findByRole('alert')
    expect(alert).toHaveTextContent('无法刷新')
    expect(within(drawer).getByRole('button', { name: '保存修改' })).toBeInTheDocument()
  })

  it('marks a device deleted by another administrator as gone and offers no actions', async () => {
    let gone = false
    const { queryClient } = renderApp('/devices?device=1', {
      api: drawerApi({}, () =>
        gone
          ? { status: 404, body: errorEnvelope('not_found') }
          : { body: deviceDetail(ALPHA, { connections: [7] }) },
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    expect(within(drawer).getByRole('button', { name: '断开连接 7' })).toBeInTheDocument()

    gone = true
    await poll(queryClient)

    expect(
      await within(drawer).findByText('该设备已不存在，可能已被其他管理员删除。'),
    ).toBeVisible()
    expect(within(drawer).queryByRole('button', { name: '断开连接 7' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除设备' })).not.toBeInTheDocument()
  })
})
