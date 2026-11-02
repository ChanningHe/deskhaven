import { createFileRoute } from '@tanstack/react-router'

import { StrategiesPage } from '@/features/strategies/components/strategies-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/strategies')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'strategies')
  },
  component: StrategiesPage,
})
