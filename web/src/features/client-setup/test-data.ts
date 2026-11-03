import type { ClientConfigView } from '@/api/generated/model'

export const CLIENT_CONFIG_PATH = '/admin/api/v1/client-config'

export const CONFIG_STRING = '9JSPNdFbxRWVQVHS5JHc6hEeF9mTGdXbVZ3aalDeyRn'
export const PUBLIC_KEY = '5Qbwsde3unUcJBtrx9ZkvUmwFNoExHzpryHuPUdqlWM='

export function clientConfig(overrides: Partial<ClientConfigView> = {}): ClientConfigView {
  return {
    id_server: 'rd.example.com',
    api_server: 'https://rd.example.com',
    public_key: PUBLIC_KEY,
    relay_servers: [],
    host_source: 'public_url',
    config_string: CONFIG_STRING,
    qr: { size: 29, path: 'M4 4h7v1h-7z' },
    windows_filename_plain: null,
    windows_filename_encoded: `rustdesk-licensed-${CONFIG_STRING}.exe`,
    ...overrides,
  }
}
