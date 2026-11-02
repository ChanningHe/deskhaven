import i18n from 'i18next'
import { describe, expect, it } from 'vitest'

import en from '@/i18n/locales/en.json'
import zhCN from '@/i18n/locales/zh-CN.json'
import { HttpError } from '@/lib/http-error'

import { ApiError } from './api-error'
import { errorCodeOf, errorMessage } from './error-message'
import { ErrorCode } from './generated/model'

describe('errorMessage', () => {
  it('has a translation for every API error code in every locale', () => {
    for (const code of Object.values(ErrorCode)) {
      expect(en.errors.codes[code]).toBeTruthy()
      expect(zhCN.errors.codes[code]).toBeTruthy()
    }
  })

  it('translates by code and never shows the server message', () => {
    const error = new ApiError(403, { code: 'forbidden', message: 'role admin required' })

    expect(errorMessage(error, i18n.t)).toBe('你没有执行此操作的权限。')
  })

  it('includes the Retry-After wait for rate limiting', () => {
    const error = new ApiError(429, { code: 'rate_limited', retryAfterSeconds: 30 })

    expect(errorMessage(error, i18n.t)).toBe('请求过于频繁，请在 30 秒后重试。')
    expect(errorMessage(new ApiError(429, { code: 'rate_limited' }), i18n.t)).toBe(
      '请求过于频繁，请稍后重试。',
    )
  })

  it('falls back to the HTTP status when there is no known code', () => {
    expect(errorCodeOf(new ApiError(502))).toBe('unavailable')
    expect(errorCodeOf(new HttpError(404))).toBe('not_found')
    expect(errorMessage(new ApiError(418), i18n.t)).toBe('发生未知错误，请重试。')
  })

  it('reports errors without a status as network failures', () => {
    expect(errorMessage(new TypeError('Failed to fetch'), i18n.t)).toBe(
      '无法连接到服务器。请检查网络后重试。',
    )
  })
})
