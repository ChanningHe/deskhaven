import type { MyAccount, MyAddressBook, MyDevice, MySession } from '@/api/generated/model'

export const API = '/admin/api/v1'
export const ACCOUNT_PATH = `${API}/me/account`
export const SESSIONS_PATH = `${API}/me/sessions`
export const DEVICES_PATH = `${API}/me/devices`
export const BOOKS_PATH = `${API}/me/address-books`

export function account(overrides: Partial<MyAccount> = {}): MyAccount {
  return {
    id: 7,
    username: 'alice',
    display_name: 'Alice',
    email: 'alice@example.test',
    email_verified: true,
    auth_source: 'local',
    role: 'user',
    totp_enabled: false,
    recovery_codes_left: 0,
    can_change_password: true,
    can_use_totp: true,
    tfa_required: false,
    last_login_at: 1_800_000_000_000,
    created_at: 1_700_000_000_000,
    ...overrides,
  }
}

export function session(id: number, overrides: Partial<MySession> = {}): MySession {
  return {
    id,
    kind: 'console',
    current: false,
    rustdesk_id: null,
    device_name: null,
    device_os: null,
    user_agent: 'Firefox 140',
    ip: '203.0.113.9',
    created_at: 1_790_000_000_000,
    last_used_at: 1_800_000_000_000,
    expires_at: 1_900_000_000_000,
    ...overrides,
  }
}

export function device(rustdeskId: string, overrides: Partial<MyDevice> = {}): MyDevice {
  return {
    rustdesk_id: rustdeskId,
    hostname: `host-${rustdeskId}`,
    username: 'alice',
    os: 'Linux',
    version: '1.4.1',
    status: 'active',
    online: false,
    last_seen_at: 1_800_000_000_000,
    ...overrides,
  }
}

export function book(name: string, overrides: Partial<MyAddressBook> = {}): MyAddressBook {
  return {
    guid: `guid-${name}`,
    kind: 'shared',
    name,
    note: '',
    owner: null,
    rule: 1,
    ...overrides,
  }
}
