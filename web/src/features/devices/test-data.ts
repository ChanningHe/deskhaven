// Fixtures of the devices and device groups page tests.

import type { DeviceDetail, DeviceGroupView, DeviceView } from '@/api/generated/model'

export const DEVICES_PATH = '/admin/api/v1/devices'
export const DEVICE_GROUPS_PATH = '/admin/api/v1/device-groups'

export function deviceView(
  id: number,
  rustdeskId: string,
  patch: Partial<DeviceView> = {},
): DeviceView {
  return {
    id,
    rustdesk_id: rustdeskId,
    hostname: 'alpha',
    username: 'alice',
    os: 'linux / Ubuntu 24.04',
    cpu: 'Intel, 3.2GHz, 8/4 cores',
    memory: '15.5GB',
    version: '1.4.1',
    owner: null,
    device_group: null,
    note: '',
    status: 'active',
    online: false,
    last_seen_at: 1_700_000_000_000,
    last_ip: '192.0.2.7',
    first_seen_at: 1_690_000_000_000,
    created_at: 1_690_000_000_000,
    ...patch,
  }
}

export function deviceDetail(device: DeviceView, patch: Partial<DeviceDetail> = {}): DeviceDetail {
  return {
    device,
    register_ip: '192.0.2.8',
    register_at: 1_699_000_000_000,
    connections: [],
    strategy: { modified_at: 1, layers: [], options: [] },
    recent_connections: [],
    ...patch,
  }
}

export function deviceGroupView(
  id: number,
  name: string,
  patch: Partial<DeviceGroupView> = {},
): DeviceGroupView {
  return {
    id,
    name,
    note: '',
    managed_by: 'ui',
    device_count: 0,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}
