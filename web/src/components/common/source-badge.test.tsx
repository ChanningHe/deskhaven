import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { SourceBadge } from './source-badge'
import type { ValueSource } from './source-badge'

function badgeFor(source: ValueSource) {
  const view = renderWithProviders(<SourceBadge source={source} />)
  return { ...view, badge: screen.getByRole('button') }
}

describe('SourceBadge', () => {
  it('labels each source with its detail as the accessible name', () => {
    const cases: [ValueSource, string, string][] = [
      [{ kind: 'default' }, '默认', '默认: 使用默认值'],
      [{ kind: 'modified' }, '已修改', '已修改: 已在控制台修改'],
      [
        { kind: 'file', path: '/etc/deskhaven.toml' },
        '配置锁定',
        '配置锁定: 由配置文件 /etc/deskhaven.toml 锁定',
      ],
      [{ kind: 'file' }, '配置锁定', '配置锁定: 由配置文件锁定'],
      [
        { kind: 'env', name: 'DESKHAVEN_SMTP_HOST' },
        '配置锁定',
        '配置锁定: 由环境变量 DESKHAVEN_SMTP_HOST 锁定',
      ],
    ]
    for (const [source, text, name] of cases) {
      const { badge, unmount } = badgeFor(source)
      expect(badge).toHaveTextContent(text)
      expect(badge).toHaveAccessibleName(name)
      unmount()
    }
  })

  it('names who changed a value and when', () => {
    const { badge } = badgeFor({ kind: 'modified', by: 'alice', at: '2026-11-03T08:30:00Z' })

    expect(badge.getAttribute('aria-label')).toMatch(/^已修改: 由 alice 于 .*2026.* 修改$/)
  })

  it('is reachable by keyboard and shows the detail in a tooltip', async () => {
    const { user, badge } = badgeFor({ kind: 'env', name: 'DESKHAVEN_SMTP_HOST' })

    await user.tab()
    expect(badge).toHaveFocus()
    await user.hover(badge)

    // Base UI tooltips are visual only (no role); the aria-label carries the same text.
    expect(await screen.findByText('由环境变量 DESKHAVEN_SMTP_HOST 锁定')).toBeVisible()
  })
})
