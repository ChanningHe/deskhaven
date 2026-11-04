import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import {
  DEVICE_GROUPS_PATH,
  DEVICES_PATH,
  deviceGroupView,
  deviceView,
} from '@/features/devices/test-data'
import { pageOf } from '@/features/users/test-data'
import { ADMIN_ME } from '@/test/fixtures'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import {
  CATALOG,
  CATALOG_PATH,
  effectiveView,
  PREVIEW_PATH,
  STRATEGIES_PATH,
  strategyView,
} from '../test-data'

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
  it('lists strategies and creates one, opening it', async () => {
    let posted: unknown
    const { user, router } = renderApp('/strategies', {
      ...base({
        [`POST ${STRATEGIES_PATH}`]: ({ init }) => {
          posted = body(init)
          return { status: 201, body: strategyView(9, 'lab') }
        },
        [`GET ${STRATEGIES_PATH}/9`]: { body: strategyView(9, 'lab') },
      }),
    })

    const row = within((await screen.findByText('kiosk')).closest('tr') as HTMLElement)
    expect(row.getByText('3')).toBeInTheDocument()
    expect(row.getByText('2')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '新建策略' }))
    const dialog = await screen.findByRole('dialog', { name: '新建策略' })
    await user.type(within(dialog).getByLabelText('名称'), 'lab')
    await user.click(within(dialog).getByRole('button', { name: '创建策略' }))

    await waitFor(() => {
      expect(router.state.location.search).toMatchObject({ strategy: 9 })
    })
    expect(posted).toEqual({ name: 'lab', note: '' })
  })

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
    expect(screen.getByText('客户端默认：开启')).toBeInTheDocument()
    expect(screen.getByText('已保存（不再显示）')).toBeInTheDocument()
    expect(screen.queryByDisplayValue('hunter2')).not.toBeInTheDocument()
    await user.selectOptions(keyboard, 'set:Y')

    const advanced = screen.getByRole('group', { name: '高级' })
    await user.click(within(advanced).getByRole('button', { name: '移除 custom-x' }))
    await user.type(within(advanced).getByLabelText('选项名'), 'bad key')
    await user.click(within(advanced).getByRole('button', { name: '添加' }))
    expect(within(advanced).getByRole('alert')).toHaveTextContent('只能包含字母')
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
    expect(await within(list).findByText('全局')).toBeInTheDocument()
    expect(within(list).getByText('设备组：Office')).toBeInTheDocument()

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

  it('previews the merged result of a device with sources and withheld secrets', async () => {
    let asked: URL | undefined
    const preview = effectiveView({
      layers: [{ scope: 'device_group', target_id: 3, strategy_id: 1, strategy_name: 'kiosk' }],
      options: [
        {
          key: 'enable-keyboard',
          value: 'N',
          redacted: false,
          source: { scope: 'device_group', target_id: 3, strategy_id: 1, strategy_name: 'kiosk' },
        },
        {
          key: 'proxy-password',
          value: '',
          redacted: true,
          source: { scope: 'device_group', target_id: 3, strategy_id: 1, strategy_name: 'kiosk' },
        },
      ],
    })
    const { user } = renderApp('/strategies?strategy=1&tab=preview', {
      ...base({
        [`GET ${DEVICES_PATH}`]: pageOf([deviceView(7, '100200300', { hostname: 'alpha' })]),
        [`GET ${PREVIEW_PATH}`]: ({ url }) => {
          asked = url
          return { body: preview }
        },
      }),
    })

    await screen.findByRole('option', { name: '100200300（alpha）' })
    await user.selectOptions(screen.getByRole('combobox', { name: '设备' }), '7')

    const merged = await screen.findByRole('table', { name: '合并后的选项' })
    expect(asked?.searchParams.get('device_id')).toBe('7')
    const keyboard = within(
      within(merged).getByText('enable-keyboard').closest('tr') as HTMLElement,
    )
    expect(keyboard.getByText('N')).toBeInTheDocument()
    expect(keyboard.getByText('设备组：kiosk')).toBeInTheDocument()
    const secret = within(within(merged).getByText('proxy-password').closest('tr') as HTMLElement)
    expect(secret.getByText('（已隐藏）')).toBeInTheDocument()
  })
})
