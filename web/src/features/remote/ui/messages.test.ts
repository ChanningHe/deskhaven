import i18n from 'i18next'
import { describe, expect, it } from 'vitest'

import { ApiError } from '@/api/api-error'
import type { ErrorCode } from '@/api/generated/model'

import { endingMessage, failureMessage } from './messages'

const t = i18n.t.bind(i18n)

function apiFailure(status: number, code?: ErrorCode, details?: Record<string, unknown>) {
  return { kind: 'api' as const, error: new ApiError(status, { code, details }) }
}

describe('failureMessage', () => {
  it.each([
    [403, 'web_client_disabled', '此服务器未开启浏览器远程。管理员可在“设置”中开启。'],
    [409, 'device_offline', '设备离线。请启动设备，或等它重新连上服务器。'],
    [504, 'device_no_answer', '设备未及时响应连接请求。请确认设备在线后重试。'],
    [409, 'no_relay', '此服务器未配置中继服务器，浏览器无法连接设备。'],
    [429, 'too_many_sessions', '你打开的远程会话过多。请先在其他标签页关闭一个再重试。'],
    [429, 'rate_limited', '连接尝试过于频繁。请等一分钟后再试。'],
    [503, 'unavailable', '此部署中信令服务器未运行。请稍后再试。'],
    [404, 'not_found', '设备不存在，或你无权控制它。'],
  ] as const)('explains %i %s', (status, code, text) => {
    expect(failureMessage(apiFailure(status, code), t)).toBe(text)
  })

  it('names the relay the device uses instead of ours', () => {
    const failure = apiFailure(409, 'foreign_relay', { relay_server: 'relay.other.example' })
    expect(failureMessage(failure, t)).toBe(
      '设备被设置为使用另一台中继服务器（relay.other.example），浏览器无法连接。' +
        '请在设备的网络设置中清空中继服务器，或改为本服务器的中继。',
    )
  })

  it('falls back to the generic message of other errors', () => {
    expect(failureMessage(apiFailure(500, 'internal'), t)).toBe(t('errors.codes.internal'))
    expect(failureMessage({ kind: 'api', error: new TypeError('fetch') }, t)).toBe(
      t('errors.network'),
    )
  })

  it('explains a browser without a decoder and a session that cannot start', () => {
    expect(failureMessage({ kind: 'no_decoder' }, t)).toContain('WebCodecs')
    expect(failureMessage({ kind: 'error', reason: 'bad key' }, t)).toBe('无法启动会话：bad key')
  })
})

describe('endingMessage', () => {
  it.each([
    ['closed', '你已结束会话。'],
    ['timeout', '设备未在规定时间内完成连接。'],
    [
      'relay connection closed (code 1006)',
      '与中继服务器的连接已断开：relay connection closed (code 1006)',
    ],
    ['relay connection failed', '与中继服务器的连接已断开：relay connection failed'],
    ['connection closed', '与中继服务器的连接已断开：connection closed'],
    ['Closed manually by the peer', '会话已结束：Closed manually by the peer'],
    ['Offline', '会话已结束：Offline'],
  ])('explains %s', (reason, text) => {
    expect(endingMessage(reason, t)).toBe(text)
  })
})
