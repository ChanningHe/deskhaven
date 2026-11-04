// Fixtures of the overview page tests.

import type { OverviewView } from '@/api/generated/model'
import { alarmView } from '@/features/audit/test-data'

export const OVERVIEW_PATH = '/admin/api/v1/overview'

export function overviewView(patch: Partial<OverviewView> = {}): OverviewView {
  return {
    generated_at: 1_700_000_000_000,
    devices: { online: 12, total: 40 },
    active_connections: 3,
    connections_24h: 1234,
    failed_logins_24h: 7,
    recent_alarms: [alarmView(2), alarmView(1, { typ: 0, info: [] })],
    client_versions: {
      versions: [
        { version: '1.4.1', count: 30 },
        { version: '1.3.0', count: 6 },
      ],
      other: 3,
      unknown: 1,
    },
    ...patch,
  }
}
