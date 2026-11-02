import { createFileRoute } from '@tanstack/react-router'

import { ClientSetupPage } from '@/features/client-setup/components/client-setup-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/client-setup')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'clientSetup')
  },
  component: ClientSetupPage,
})
