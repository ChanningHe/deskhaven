// Fixtures of the strategy page tests.

import type {
  CatalogOptionView,
  CatalogView,
  EffectiveStrategyView,
  OptionKindView,
  StrategyView,
} from '@/api/generated/model'

export const STRATEGIES_PATH = '/admin/api/v1/strategies'
export const CATALOG_PATH = '/admin/api/v1/strategy-catalog'
export const PREVIEW_PATH = '/admin/api/v1/strategy-preview'

export function strategyView(
  id: number,
  name: string,
  patch: Partial<StrategyView> = {},
): StrategyView {
  return {
    id,
    name,
    note: '',
    managed_by: 'ui',
    options: {},
    secret_options: [],
    options_corrupt: false,
    assignment_count: 0,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

export function catalogOption(
  key: string,
  kind: OptionKindView,
  patch: Partial<CatalogOptionView> = {},
): CatalogOptionView {
  return {
    key,
    kind,
    group: 'security',
    default: null,
    secret: false,
    choices: [],
    min: null,
    max: null,
    label: { en: key, zh: key },
    description: { en: '', zh: '' },
    ...patch,
  }
}

export const CATALOG: CatalogView = {
  source: { rustdesk: '23e5bb4', hbb_common: 'fae47e0' },
  options: [
    catalogOption('approve-mode', 'choice', {
      default: 'password-click',
      label: { en: 'Session approval', zh: '会话接受方式' },
      choices: [
        { value: 'password', label: { en: 'Via password', zh: '通过密码接受会话' } },
        { value: 'click', label: { en: 'Via click', zh: '通过点击接受会话' } },
      ],
    }),
    catalogOption('enable-keyboard', 'bool', {
      group: 'permissions',
      default: 'Y',
      label: { en: 'Keyboard and mouse', zh: '键盘和鼠标' },
    }),
    catalogOption('direct-access-port', 'number', {
      default: '21118',
      min: 1,
      max: 65535,
      label: { en: 'Direct access port', zh: '直接访问端口' },
    }),
    catalogOption('proxy-password', 'text', {
      group: 'network',
      secret: true,
      label: { en: 'Proxy password', zh: '代理密码' },
    }),
  ],
}

export function effectiveView(patch: Partial<EffectiveStrategyView> = {}): EffectiveStrategyView {
  return { modified_at: 1_700_000_000_500, layers: [], options: [], ...patch }
}
