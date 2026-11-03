import { act, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { getGetClientConfigQueryKey } from '@/api/generated/client-config/client-config'
import type { ClientConfigView } from '@/api/generated/model'
import { ADMIN_ME, USER_ME } from '@/test/fixtures'
import { errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { CLIENT_CONFIG_PATH, clientConfig, CONFIG_STRING, PUBLIC_KEY } from '../test-data'

async function openPage(view: ClientConfigView = clientConfig()) {
  const app = renderApp('/client-setup', {
    me: ADMIN_ME,
    api: { [`GET ${CLIENT_CONFIG_PATH}`]: { body: view } },
  })
  await screen.findByRole('heading', { name: '服务器地址' })
  return app
}

describe('client setup page', () => {
  it('shows the addresses, the key and the configuration string', async () => {
    await openPage()

    expect(screen.getByRole('textbox', { name: 'ID 服务器' })).toHaveValue('rd.example.com')
    expect(screen.getByRole('textbox', { name: 'API 服务器' })).toHaveValue(
      'https://rd.example.com',
    )
    expect(screen.getByRole('textbox', { name: 'Key（公钥）' })).toHaveValue(PUBLIC_KEY)
    expect(screen.getByRole('textbox', { name: '配置字符串' })).toHaveValue(CONFIG_STRING)
    expect(screen.queryByRole('note')).not.toBeInTheDocument()
  })

  it('copies a value with its own button', async () => {
    const { user } = await openPage()

    await user.click(screen.getByRole('button', { name: '复制 配置字符串' }))

    expect(await screen.findByText('已复制到剪贴板')).toBeInTheDocument()
    await expect(navigator.clipboard.readText()).resolves.toBe(CONFIG_STRING)
  })

  it('draws the QR code from the server path under an accessible label', async () => {
    await openPage()

    const qr = screen.getByRole('img', { name: 'RustDesk 客户端配置二维码' })
    expect(qr).toHaveAttribute('viewBox', '0 0 29 29')
    expect(qr.querySelector('path')).toHaveAttribute('d', 'M4 4h7v1h-7z')
  })

  it('lists the Windows file name and says why the readable one is missing', async () => {
    await openPage()

    expect(screen.getByRole('textbox', { name: '完整文件名' })).toHaveValue(
      `rustdesk-licensed-${CONFIG_STRING}.exe`,
    )
    expect(screen.queryByRole('textbox', { name: '可读文件名' })).not.toBeInTheDocument()
    expect(screen.getByText(/Windows 文件名无法包含/)).toBeInTheDocument()
  })

  it('shows the readable Windows file name when there is one', async () => {
    await openPage(
      clientConfig({ windows_filename_plain: 'rustdesk-host=rd.example.com,key=k,.exe' }),
    )

    expect(screen.getByRole('textbox', { name: '可读文件名' })).toHaveValue(
      'rustdesk-host=rd.example.com,key=k,.exe',
    )
  })

  it('gives command examples with the string and an API token placeholder', async () => {
    await openPage()

    const config = screen.getByRole('textbox', { name: '应用服务器设置' })
    expect(config).toHaveValue(`rustdesk --config ${CONFIG_STRING}`)
    const deploy = screen.getByRole('textbox', { name: '登记设备（deploy）' })
    expect(deploy).toHaveValue('rustdesk --deploy --token dhk_YOUR_API_TOKEN')
    const assign = screen.getByRole('textbox', { name: '分配设备（assign）' })
    expect((assign as HTMLTextAreaElement).value).toContain('--assign --token dhk_YOUR_API_TOKEN')
  })

  it('warns when the ID server comes from the address the console was opened with', async () => {
    await openPage(clientConfig({ host_source: 'request_host', api_server: '' }))

    const warning = screen.getByRole('note')
    expect(within(warning).getByText(/public_url/)).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'API 服务器' })).toHaveValue('')
  })

  it('offers no string, QR code or file name without an ID server address', async () => {
    await openPage(
      clientConfig({
        host_source: 'unknown',
        id_server: '',
        config_string: null,
        qr: null,
        windows_filename_encoded: null,
      }),
    )

    expect(screen.getByRole('note')).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '配置字符串' })).not.toBeInTheDocument()
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Key（公钥）' })).toHaveValue(PUBLIC_KEY)
  })

  it('lists the configured relays for information only', async () => {
    await openPage(clientConfig({ relay_servers: ['a.example.com:21117', 'b.example.com:21117'] }))

    expect(screen.getByRole('textbox', { name: '中继服务器列表' })).toHaveValue(
      'a.example.com:21117, b.example.com:21117',
    )
    expect(screen.getByText(/不写入配置字符串/)).toBeInTheDocument()
  })

  it('says so when no relay is configured', async () => {
    await openPage()

    expect(screen.getByRole('textbox', { name: '中继服务器列表' })).toHaveValue('')
    expect(screen.getByText(/未配置/)).toBeInTheDocument()
  })

  it('forgets that a value was copied when the value changes', async () => {
    const { user, queryClient } = await openPage()
    await user.click(screen.getByRole('button', { name: '复制 配置字符串' }))
    expect(await screen.findByText('已复制到剪贴板')).toBeInTheDocument()

    await act(async () => {
      queryClient.setQueryData(
        getGetClientConfigQueryKey(),
        clientConfig({ config_string: 'OTHERSTRING=' }),
      )
      await Promise.resolve()
    })

    expect(await screen.findByRole('textbox', { name: '配置字符串' })).toHaveValue('OTHERSTRING=')
    expect(screen.queryByText('已复制到剪贴板')).not.toBeInTheDocument()
  })

  it('shows an API error with a retry', async () => {
    renderApp('/client-setup', {
      me: ADMIN_ME,
      api: {
        [`GET ${CLIENT_CONFIG_PATH}`]: { status: 500, body: errorEnvelope('internal_error') },
      },
    })

    expect(await screen.findByRole('button', { name: /重试/ })).toBeInTheDocument()
  })

  it('is not for plain users', async () => {
    renderApp('/client-setup', { me: USER_ME })

    expect(await screen.findByText('无权访问')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: '服务器地址' })).not.toBeInTheDocument()
  })
})
