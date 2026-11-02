import { createFileRoute } from '@tanstack/react-router'

import { DevicesPage } from '@/features/devices/components/devices-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/devices')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'devices')
  },
  component: DevicesPage,
})
