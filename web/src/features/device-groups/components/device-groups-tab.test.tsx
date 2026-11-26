import { act, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { getGetDeviceGroupQueryKey } from '@/api/generated/device-groups/device-groups'
import { errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import { DEVICE_GROUPS_PATH, deviceGroupView } from '@/features/devices/test-data'
import { pageOf } from '@/features/users/test-data'

const OFFICE = deviceGroupView(3, 'Office', { device_count: 4, note: 'HQ floor' })

describe('device groups tab', () => {
  it('saves only the edited field when another admin changed the other one meanwhile', async () => {
    let served = OFFICE
    const bodies: unknown[] = []
    const { user, queryClient } = renderApp('/groups?tab=devices&group=3', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
        [`GET ${DEVICE_GROUPS_PATH}/3`]: () => ({ body: served }),
        [`PATCH ${DEVICE_GROUPS_PATH}/3`]: ({ init }) => {
          bodies.push(JSON.parse(init.body as string))
          served = { ...served, name: 'HQ' }
          return { body: served }
        },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'Office' })
    const name = within(drawer).getByLabelText('名称')
    await user.clear(name)
    await user.type(name, 'HQ')

    served = { ...OFFICE, note: 'moved to floor 2' }
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: getGetDeviceGroupQueryKey(3) })
    })
    await waitFor(() => {
      expect(within(drawer).getByLabelText('备注')).toHaveValue('moved to floor 2')
    })
    expect(name).toHaveValue('HQ')

    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ name: 'HQ' }])
    })
    expect(within(drawer).getByLabelText('备注')).toHaveValue('moved to floor 2')
  })

  it('keeps the form when a refresh fails, and drops it when another admin deleted the group', async () => {
    let failure: { status: number; body: unknown } | undefined
    const { queryClient } = renderApp('/groups?tab=devices&group=3', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
        [`GET ${DEVICE_GROUPS_PATH}/3`]: () => failure ?? { body: OFFICE },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'Office' })
    const refetch = () =>
      act(async () => {
        await queryClient.refetchQueries({ queryKey: getGetDeviceGroupQueryKey(3) })
      })

    failure = { status: 429, body: errorEnvelope('rate_limited') }
    await refetch()
    expect(await within(drawer).findByRole('alert')).toBeInTheDocument()
    expect(within(drawer).getByLabelText('名称')).toBeInTheDocument()

    failure = { status: 404, body: errorEnvelope('not_found') }
    await refetch()
    await waitFor(() => {
      expect(within(drawer).queryByLabelText('名称')).not.toBeInTheDocument()
    })
    expect(within(drawer).queryByRole('button', { name: '删除设备组' })).not.toBeInTheDocument()
  })

  it('shows a config-managed group read-only', async () => {
    const fleet = deviceGroupView(4, 'fleet', { managed_by: 'config' })
    renderApp('/groups?tab=devices&group=4', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([fleet]),
        [`GET ${DEVICE_GROUPS_PATH}/4`]: { body: fleet },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'fleet' })
    expect(within(drawer).queryByLabelText('名称')).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除设备组' })).not.toBeInTheDocument()
  })
})
