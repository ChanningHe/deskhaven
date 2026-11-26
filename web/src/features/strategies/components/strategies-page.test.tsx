import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { DEVICE_GROUPS_PATH, deviceGroupView } from '@/features/devices/test-data'
import { pageOf } from '@/features/users/test-data'
import { ADMIN_ME } from '@/test/fixtures'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { CATALOG, CATALOG_PATH, STRATEGIES_PATH, strategyView } from '../test-data'

const KIOSK = strategyView(1, 'kiosk', {
  options: { 'enable-keyboard': 'N', 'custom-x': '1' },
  secret_options: ['proxy-password'],
  assignment_count: 2,
})
const KIOSK_PATH = `${STRATEGIES_PATH}/1`
const ASSIGNMENTS_PATH = `${KIOSK_PATH}/assignments`

function base(extra: Record<string, MockHandler | { body?: unknown; status?: number }> = {}) {
  return {
    me: ADMIN_ME,
    api: {
      [`GET ${STRATEGIES_PATH}`]: pageOf([KIOSK]),
      [`GET ${CATALOG_PATH}`]: { body: CATALOG },
      [`GET ${KIOSK_PATH}`]: { body: KIOSK },
      ...extra,
    },
  }
}

function body(init: RequestInit): unknown {
  return JSON.parse(init.body as string)
}

describe('strategies page', () => {
  it('edits typed and free options and saves only what changed', async () => {
    let patched: unknown
    const { user } = renderApp('/strategies?strategy=1', {
      ...base({
        [`PATCH ${KIOSK_PATH}`]: ({ init }) => {
          patched = body(init)
          return { body: { ...KIOSK, options: { 'enable-keyboard': 'Y', 'custom-y': '2' } } }
        },
      }),
    })

    const keyboard = await screen.findByRole('combobox', { name: '键盘和鼠标' })
    expect(keyboard).toHaveValue('set:N')
    await user.selectOptions(keyboard, 'set:Y')

    const advanced = screen.getByRole('group', { name: '高级' })
    await user.click(within(advanced).getByRole('button', { name: '移除 custom-x' }))
    await user.type(within(advanced).getByLabelText('选项名'), 'bad key')
    await user.click(within(advanced).getByRole('button', { name: '添加' }))
    expect(within(advanced).getByRole('alert')).toBeInTheDocument()
    await user.clear(within(advanced).getByLabelText('选项名'))
    await user.type(within(advanced).getByLabelText('选项名'), 'custom-y')
    await user.type(within(advanced).getByLabelText('值'), '2')
    await user.click(within(advanced).getByRole('button', { name: '添加' }))

    await user.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() => {
      expect(patched).toEqual({
        options: { 'enable-keyboard': 'Y', 'custom-x': null, 'custom-y': '2' },
      })
    })
  })

  it('keeps custom-client options out of the form and refuses an empty "set to"', async () => {
    const { user } = renderApp('/strategies?strategy=1', {
      ...base({
        [`GET ${KIOSK_PATH}`]: {
          body: { ...KIOSK, options_corrupt: true, options: {}, secret_options: [] },
        },
      }),
    })

    const whitelist = await screen.findByRole('combobox', { name: 'IP 白名单' })
    expect(screen.queryByRole('combobox', { name: '代理密码' })).not.toBeInTheDocument()
    await user.selectOptions(whitelist, 'set')

    expect(screen.getByRole('button', { name: '保存' })).toBeDisabled()
    await user.type(screen.getByLabelText('IP 白名单 的值'), '10.0.0.0/8')
    expect(screen.getByRole('button', { name: '保存' })).toBeEnabled()
  })

  it('shows withheld secrets as password inputs when the catalog fails to load', async () => {
    const { user } = renderApp('/strategies?strategy=1', {
      ...base({ [`GET ${CATALOG_PATH}`]: { status: 500, body: { error: { code: 'internal' } } } }),
    })

    const advanced = await screen.findByRole('group', { name: '高级' })
    await user.click(await within(advanced).findByRole('button', { name: '替换' }))

    expect(within(advanced).getByLabelText('proxy-password 的值')).toHaveAttribute(
      'type',
      'password',
    )
    expect(within(advanced).getByLabelText('custom-x 的值')).toHaveAttribute('type', 'text')
  })

  it('asks before assigning globally', async () => {
    let assigned: unknown
    const { user } = renderApp('/strategies?strategy=1&tab=assignments', {
      ...base({
        [`GET ${ASSIGNMENTS_PATH}`]: { body: { items: [] } },
        [`POST ${ASSIGNMENTS_PATH}`]: ({ init }) => {
          assigned = body(init)
          return { status: 204 }
        },
      }),
    })

    await user.click(await screen.findByRole('button', { name: '分配' }))
    const confirm = await screen.findByRole('alertdialog')
    expect(assigned).toBeUndefined()
    await user.click(within(confirm).getByRole('button', { name: '全局分配' }))

    await waitFor(() => {
      expect(assigned).toEqual({ target_type: 'global' })
    })
  })

  it('assigns the strategy to a device group and removes an assignment', async () => {
    let assigned: unknown
    let removed: URL | undefined
    const { user } = renderApp('/strategies?strategy=1&tab=assignments', {
      ...base({
        [`GET ${ASSIGNMENTS_PATH}`]: {
          body: {
            items: [
              { target_type: 'global', target_id: null, target_name: null },
              { target_type: 'device_group', target_id: 3, target_name: 'Office' },
            ],
          },
        },
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([
          deviceGroupView(3, 'Office'),
          deviceGroupView(4, 'Lab'),
        ]),
        [`POST ${ASSIGNMENTS_PATH}`]: ({ init }) => {
          assigned = body(init)
          return { status: 204 }
        },
        [`DELETE ${ASSIGNMENTS_PATH}`]: ({ url }) => {
          removed = url
          return { status: 204 }
        },
      }),
    })

    const list = await screen.findByRole('list', { name: '分配对象' })
    await within(list).findByRole('button', { name: '移除 设备组：Office' })

    await user.selectOptions(screen.getByRole('combobox', { name: '分配对象类型' }), 'device_group')
    await user.selectOptions(await screen.findByRole('combobox', { name: '设备组' }), '4')
    await user.click(screen.getByRole('button', { name: '分配' }))
    await waitFor(() => {
      expect(assigned).toEqual({ target_type: 'device_group', target_id: 4 })
    })

    await user.click(within(list).getByRole('button', { name: '移除 设备组：Office' }))
    const confirm = await screen.findByRole('alertdialog')
    await user.click(within(confirm).getByRole('button', { name: '移除' }))
    await waitFor(() => {
      expect(removed?.searchParams.get('target_type')).toBe('device_group')
    })
    expect(removed?.searchParams.get('target_id')).toBe('3')
  })
})
