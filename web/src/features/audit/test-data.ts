// Fixtures of the audit page tests.

import type {
  AlarmAuditView,
  ConnectionAuditView,
  ConsoleAuditView,
  FileAuditView,
  LoginAuditView,
} from '@/api/generated/model'

export const AUDIT_PATH = '/admin/api/v1/audit'
export const CONNECTIONS_PATH = `${AUDIT_PATH}/connections`
export const FILES_PATH = `${AUDIT_PATH}/files`
export const ALARMS_PATH = `${AUDIT_PATH}/alarms`
export const LOGINS_PATH = `${AUDIT_PATH}/logins`
export const CONSOLE_PATH = `${AUDIT_PATH}/console`

const T0 = 1_700_000_000_000

export function connectionView(
  id: number,
  patch: Partial<ConnectionAuditView> = {},
): ConnectionAuditView {
  return {
    id,
    guid: `guid-${String(id)}`,
    device_id: '100200300',
    conn_id: id,
    session_id: String(1000 + id),
    peer_id: '900800700',
    peer_name: 'Reception PC',
    controller: { id: 5, username: 'bob' },
    conn_type: 0,
    primary_auth: 2,
    two_factor: null,
    ip: '198.51.100.4',
    started_at: T0,
    authorized_at: T0 + 1000,
    ended_at: T0 + 60_000,
    note: null,
    controller_note: null,
    created_at: T0 + id * 1000,
    ...patch,
  }
}

export function fileView(id: number, patch: Partial<FileAuditView> = {}): FileAuditView {
  return {
    id,
    device_id: '100200300',
    peer_id: '900800700',
    conn_id: 1,
    direction: 'send',
    path: '/home/alice/report.pdf',
    is_file: true,
    file_count: 2,
    files: [
      { name: 'report.pdf', size: 2048 },
      { name: 'big.iso', size: 3_221_225_472 },
    ],
    ip: '198.51.100.4',
    created_at: T0 + id * 1000,
    ...patch,
  }
}

export function alarmView(id: number, patch: Partial<AlarmAuditView> = {}): AlarmAuditView {
  return {
    id,
    device_id: '100200300',
    typ: 1,
    info: [
      { key: 'ip', value: '198.51.100.9' },
      { key: 'name', value: '<b>mallory</b>' },
    ],
    conn_id: 7,
    created_at: T0 + id * 1000,
    ...patch,
  }
}

export function loginView(id: number, patch: Partial<LoginAuditView> = {}): LoginAuditView {
  return {
    id,
    user: { id: 5, username: 'bob' },
    username_attempted: 'bob',
    method: 'password',
    result: 'success',
    client: 'console',
    rustdesk_id: null,
    ip: '203.0.113.9',
    created_at: T0 + id * 1000,
    ...patch,
  }
}

export function consoleView(id: number, patch: Partial<ConsoleAuditView> = {}): ConsoleAuditView {
  return {
    id,
    actor: { kind: 'user', id: 1, name: 'admin' },
    action: 'user.update',
    target_type: 'user',
    target_id: '5',
    before: { role: 'user', note: 'x' },
    after: { role: 'admin', note: 'x' },
    ip: null,
    created_at: T0 + id * 1000,
    ...patch,
  }
}
