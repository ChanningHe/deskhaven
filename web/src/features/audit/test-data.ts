// Fixtures of the audit page tests.

import type { AlarmAuditView, ConnectionAuditView } from '@/api/generated/model'

export const AUDIT_PATH = '/admin/api/v1/audit'
export const CONNECTIONS_PATH = `${AUDIT_PATH}/connections`
export const ALARMS_PATH = `${AUDIT_PATH}/alarms`

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
