import type { SettingSchema, SettingView } from '@/api/generated/model'

export const SETTINGS_PATH = '/admin/api/v1/settings'

interface Spec {
  value: unknown
  default?: unknown
  schema: SettingSchema
  category?: string
  description?: string
}

function setting(key: string, spec: Spec, patch: Partial<SettingView> = {}): SettingView {
  return {
    key,
    category: spec.category ?? key.split('.')[0] ?? key,
    description: spec.description ?? `About ${key}`,
    value: spec.value,
    default: spec.default ?? spec.value,
    source: { kind: 'default' },
    pinned: false,
    sensitive: spec.schema.type === 'secret',
    version: 0,
    schema: spec.schema,
    ...patch,
  }
}

/** A small, varied registry: one setting of every control kind. */
export function sampleSettings(): SettingView[] {
  return [
    setting('auth.session_ttl', {
      value: '30d',
      schema: { type: 'duration', min: '5m', max: '365d', zero_allowed: false },
    }),
    setting('auth.password.min_length', {
      value: 10,
      schema: { type: 'u32', min: 8, max: 256, zero_allowed: false },
    }),
    setting('auth.registration', {
      value: 'disabled',
      schema: { type: 'enum', variants: ['disabled', 'invite', 'open'] },
    }),
    setting('smtp.host', {
      value: '',
      schema: { type: 'string', max_length: 254, format: 'host' },
    }),
    setting('smtp.password', {
      value: { set: false },
      default: { set: false },
      schema: { type: 'secret', max_length: 254 },
    }),
    setting('devices.verify_uuid', { value: true, schema: { type: 'bool' } }),
    setting('signal.relay_servers', {
      value: [],
      schema: { type: 'host_port_list', max_items: 32 },
    }),
  ]
}

/** `settings` with `patch` applied to the one named `key`. */
export function withSetting(
  settings: readonly SettingView[],
  key: string,
  patch: Partial<SettingView>,
): SettingView[] {
  return settings.map((s) => (s.key === key ? { ...s, ...patch } : s))
}
