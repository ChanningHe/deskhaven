import { createFileRoute } from '@tanstack/react-router'

import { IdentityPage } from '@/features/auth/components/identity-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/identity')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'identity')
  },
  component: IdentityPage,
})
