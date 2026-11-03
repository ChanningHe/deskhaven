import { createFileRoute } from '@tanstack/react-router'

import { DevicesPage } from '@/features/devices/components/devices-page'
import { parseDevicesSearch } from '@/features/devices/devices-search'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/devices')({
  validateSearch: parseDevicesSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'devices')
  },
  component: DevicesPage,
})
